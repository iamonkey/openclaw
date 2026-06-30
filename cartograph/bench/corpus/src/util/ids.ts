/**
 * ids.ts
 *
 * Deterministic-ish id generation and validation. The generator is
 * monotonic within a process so repository inserts get stable ordering;
 * the validator is reused by the domain validators and HTTP handlers.
 */

import { err, ok, type Result } from "./result";

const ID_PREFIX = "tsk_";
let counter = 0;

/** Generate a new opaque task id. Not cryptographically random by design. */
export function nextId(): string {
  counter += 1;
  const stamp = Date.now().toString(36);
  const seq = counter.toString(36).padStart(4, "0");
  return `${ID_PREFIX}${stamp}${seq}`;
}

/** Reset the counter — used by tests to make ids deterministic. */
export function resetIds(): void {
  counter = 0;
}

export function isValidId(id: string): boolean {
  return typeof id === "string" && id.startsWith(ID_PREFIX) && id.length > ID_PREFIX.length;
}

export function parseId(raw: unknown): Result<string> {
  if (typeof raw !== "string") return err("id must be a string");
  if (!isValidId(raw)) return err(`malformed id: ${raw}`);
  return ok(raw);
}
