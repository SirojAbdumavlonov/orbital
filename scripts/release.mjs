#!/usr/bin/env node
// Prepare a release: bump the version everywhere, date the CHANGELOG, commit and tag.
//
//   npm run release -- patch          1.0.0 → 1.0.1   (fixes)
//   npm run release -- minor          1.0.1 → 1.1.0   (new features)
//   npm run release -- major          1.1.0 → 2.0.0   (users must adapt)
//   npm run release -- beta           1.1.0 → 1.1.1-beta.1, then -beta.2, …
//   npm run release -- 1.2.0          any higher version
//   add --dry-run to see what would change without writing anything
//
// What changed goes under "## Unreleased" in CHANGELOG.md before running this;
// that text becomes the release notes the app shows. Pushing the tag starts
// .github/workflows/release.yml, which builds a draft release; publishing the
// draft offers the update to everyone.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const dry = args.includes("--dry-run");
const want = args.find((a) => !a.startsWith("--"));
const die = (msg) => {
  console.error(`release: ${msg}`);
  process.exit(1);
};
const git = (...a) => execFileSync("git", a, { cwd: root, encoding: "utf8" }).trim();
const read = (f) => readFileSync(join(root, f), "utf8");
const write = (f, s) => (dry ? console.log(`would write ${f}`) : writeFileSync(join(root, f), s));

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/;

export function parse(v) {
  const m = SEMVER.exec(v);
  if (!m) return null;
  return { major: +m[1], minor: +m[2], patch: +m[3], pre: m[4] ?? null };
}

/** -1, 0 or 1. A prerelease sorts before its release (1.1.0-beta.1 < 1.1.0). */
export function compare(a, b) {
  const x = parse(a), y = parse(b);
  for (const k of ["major", "minor", "patch"]) if (x[k] !== y[k]) return x[k] < y[k] ? -1 : 1;
  if (x.pre === y.pre) return 0;
  if (x.pre === null) return 1;
  if (y.pre === null) return -1;
  const p = x.pre.split("."), q = y.pre.split(".");
  for (let i = 0; i < Math.max(p.length, q.length); i++) {
    if (p[i] === undefined) return -1;
    if (q[i] === undefined) return 1;
    const n = /^\d+$/.test(p[i]) && /^\d+$/.test(q[i]);
    if (p[i] !== q[i]) return n ? (+p[i] < +q[i] ? -1 : 1) : p[i] < q[i] ? -1 : 1;
  }
  return 0;
}

export function next(current, how) {
  const c = parse(current);
  if (!c) throw new Error(`current version ${current} isn't SemVer`);
  const base = `${c.major}.${c.minor}.${c.patch}`;
  switch (how) {
    // From a prerelease, "patch" (or minor/major when it already matches) releases it.
    case "patch": return c.pre ? base : `${c.major}.${c.minor}.${c.patch + 1}`;
    case "minor": return c.pre && c.patch === 0 ? base : `${c.major}.${c.minor + 1}.0`;
    case "major": return c.pre && c.minor === 0 && c.patch === 0 ? base : `${c.major + 1}.0.0`;
    case "beta": {
      const m = /^beta\.(\d+)$/.exec(c.pre ?? "");
      return m ? `${base}-beta.${+m[1] + 1}` : `${c.major}.${c.minor}.${c.patch + 1}-beta.1`;
    }
    default:
      return how;
  }
}

/** Turn "## Unreleased" into "## X (date)" and open a new, empty Unreleased section. */
export function dateChangelog(text, version, date) {
  const m = /^## Unreleased[^\n]*\n([\s\S]*?)(?=^## |(?![\s\S]))/m.exec(text);
  if (!m) throw new Error('CHANGELOG.md has no "## Unreleased" section');
  if (!m[1].trim()) throw new Error('nothing is written under "## Unreleased" in CHANGELOG.md');
  return text.slice(0, m.index) + `## Unreleased\n\n## ${version} (${date})\n${m[1]}` + text.slice(m.index + m[0].length);
}

function main() {
  if (!want) die("say which version: patch, minor, major, beta or a number like 1.2.0");
  const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
  const current = conf.version;
  const version = next(current, want);
  if (!parse(version)) die(`${version} isn't a version like 1.2.0 or 1.2.0-beta.1`);
  if (compare(version, current) <= 0) die(`${version} isn't newer than ${current}; installed copies would never take it`);
  const tag = `v${version}`;

  if (!dry) {
    if (git("status", "--porcelain")) die("commit or stash your changes first, so the release commit holds only the version bump");
    if (git("tag", "--list", tag)) die(`tag ${tag} already exists`);
  }

  const date = new Date().toISOString().slice(0, 10);
  let changelog;
  try {
    changelog = dateChangelog(read("CHANGELOG.md"), version, date);
  } catch (e) {
    die(e.message);
  }

  conf.version = version;
  write("src-tauri/tauri.conf.json", JSON.stringify(conf, null, 2) + "\n");

  const cargo = read("Cargo.toml");
  const cargoNew = cargo.replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+"/, `$1${version}"`);
  if (cargoNew === cargo) die("couldn't find version in [workspace.package] of Cargo.toml");
  write("Cargo.toml", cargoNew);

  // Our own crates in Cargo.lock, so the release build doesn't have to rewrite it.
  const lock = read("Cargo.lock").replace(/(\[\[package\]\]\nname = "(?:orbital|orbital-core)"\nversion = ")[^"]+"/g, `$1${version}"`);
  write("Cargo.lock", lock);

  for (const f of ["package.json", "package-lock.json"]) {
    const j = JSON.parse(read(f));
    j.version = version;
    if (j.packages?.[""]) j.packages[""].version = version;
    write(f, JSON.stringify(j, null, 2) + "\n");
  }
  write("CHANGELOG.md", changelog);

  if (dry) {
    console.log(`\n${current} → ${version} (dry run: nothing written, committed or tagged)`);
    return;
  }
  git("add", "src-tauri/tauri.conf.json", "Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "CHANGELOG.md");
  git("commit", "-m", `Release ${tag}`);
  git("tag", "-a", tag, "-m", `Orbital ${version}`);
  console.log(`\n${current} → ${version}: committed and tagged ${tag}. To build the release:\n\n  git push && git push origin ${tag}\n`);
  console.log("Then publish the draft on GitHub; installed copies pick it up within the hour.");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
