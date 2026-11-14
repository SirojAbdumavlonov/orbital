// Map a remote file path to a Monaco language id.
const BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile",
  makefile: "makefile",
  "nginx.conf": "nginx",
  caddyfile: "plaintext",
  ".env": "ini",
  ".bashrc": "shell",
  ".zshrc": "shell",
  ".profile": "shell",
  sshd_config: "ini",
  ssh_config: "ini",
  crontab: "shell",
  hosts: "plaintext",
  fstab: "plaintext",
  jenkinsfile: "groovy",
};

const BY_EXT: Record<string, string> = {
  js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
  ts: "typescript", tsx: "typescript",
  json: "json", jsonc: "json",
  yml: "yaml", yaml: "yaml",
  xml: "xml", html: "html", htm: "html", css: "css", scss: "scss", less: "less",
  md: "markdown", markdown: "markdown",
  py: "python", rb: "ruby", go: "go", rs: "rust", java: "java", kt: "kotlin", kts: "kotlin",
  scala: "scala", groovy: "groovy", gradle: "groovy", php: "php", swift: "swift", c: "c", h: "c",
  cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp", lua: "lua", pl: "perl", r: "r",
  sh: "shell", bash: "shell", zsh: "shell",
  sql: "sql", ini: "ini", cfg: "ini", conf: "ini", cnf: "ini", toml: "ini", properties: "ini",
  service: "ini", timer: "ini", socket: "ini", env: "ini",
  tf: "hcl", hcl: "hcl", proto: "proto", graphql: "graphql",
};

export function languageFor(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  if (BY_NAME[name]) return BY_NAME[name];
  if (name.startsWith("dockerfile")) return "dockerfile";
  if (name.startsWith(".env")) return "ini";
  if (name.startsWith("docker-compose") || name.startsWith("compose.")) return "yaml";
  // Anything under an nginx config tree is nginx syntax, whatever the extension.
  if (/\/(nginx|openresty)\//.test(path) && !/\.(json|ya?ml|lua|html?)$/.test(name)) return "nginx";
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  return BY_EXT[ext] ?? "plaintext";
}
