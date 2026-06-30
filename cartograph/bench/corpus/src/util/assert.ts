/**
 * assert.ts
 *
 * Lightweight invariant helpers. Used at internal boundaries where a
 * violated precondition is a programmer error (not user input). Kept
 * dependency-free so anything can import it.
 */

export class InvariantError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "InvariantError";
  }
}

export function invariant(condition: unknown, message: string): asserts condition {
  if (!condition) {
    throw new InvariantError(message);
  }
}

export function assertNever(value: never, label = "unreachable"): never {
  throw new InvariantError(`${label}: ${String(value)}`);
}

export function assertNonEmpty<T>(items: readonly T[], label: string): void {
  invariant(items.length > 0, `${label} must be non-empty`);
}
