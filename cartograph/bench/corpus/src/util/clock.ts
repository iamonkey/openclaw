/**
 * clock.ts
 *
 * A tiny injectable clock so timestamps in the domain layer are testable.
 * The repository and domain factories take a Clock instead of calling
 * Date.now() directly.
 */

export interface Clock {
  now(): number;
  iso(): string;
}

export const systemClock: Clock = {
  now() {
    return Date.now();
  },
  iso() {
    return new Date().toISOString();
  },
};

/** A frozen clock for deterministic tests. */
export function fixedClock(epochMillis: number): Clock {
  return {
    now() {
      return epochMillis;
    },
    iso() {
      return new Date(epochMillis).toISOString();
    },
  };
}
