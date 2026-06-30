/**
 * env.ts
 *
 * Thin typed accessors over a raw environment bag. Kept separate from the
 * loader so tests can pass a plain object instead of process.env.
 */

export type EnvBag = Record<string, string | undefined>;

export function readString(env: EnvBag, key: string, fallback: string): string {
  const v = env[key];
  return v === undefined || v.length === 0 ? fallback : v;
}

export function readInt(env: EnvBag, key: string, fallback: number): number {
  const v = env[key];
  if (v === undefined) return fallback;
  const n = Number.parseInt(v, 10);
  return Number.isNaN(n) ? fallback : n;
}

export function readBool(env: EnvBag, key: string, fallback: boolean): boolean {
  const v = env[key];
  if (v === undefined) return fallback;
  return v === "1" || v.toLowerCase() === "true" || v.toLowerCase() === "yes";
}

export function readEnum<T extends string>(
  env: EnvBag,
  key: string,
  allowed: readonly T[],
  fallback: T,
): T {
  const v = env[key];
  if (v !== undefined && (allowed as readonly string[]).includes(v)) {
    return v as T;
  }
  return fallback;
}
