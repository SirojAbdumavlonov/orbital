import { DnsCheck } from "./api";

export type Readiness =
  /** At least one resolver sees this server's address and none sees another. */
  | { state: "here"; lagging: string[] }
  /** A resolver sees a different address. */
  | { state: "elsewhere"; others: string[] }
  /** No record yet (or it hasn't spread to any resolver). */
  | { state: "none" }
  /** No resolver could be asked. */
  | { state: "unknown"; error: string };

/**
 * Does a name point at this server? One slow or not-yet-updated resolver
 * doesn't make it "no": a new record takes a while to reach every resolver,
 * and Let's Encrypt asks the domain's own name servers anyway.
 */
export function readiness(c: DnsCheck): Readiness {
  const expected = new Set(c.expected);
  const answered = c.answers.filter((a) => a.status === "NOERROR" && a.values.length > 0);
  const others = [...new Set(answered.flatMap((a) => a.values).filter((v) => !expected.has(v)))];
  if (others.length > 0) return { state: "elsewhere", others };
  const confirmed = answered.filter((a) => a.values.some((v) => expected.has(v)));
  if (confirmed.length > 0) {
    return { state: "here", lagging: c.answers.filter((a) => !confirmed.includes(a)).map((a) => a.resolver) };
  }
  if (c.answers.length > 0 && c.answers.every((a) => a.error)) return { state: "unknown", error: c.answers[0].error ?? "no answer" };
  return { state: "none" };
}
