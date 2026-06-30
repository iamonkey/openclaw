/**
 * strings.ts
 *
 * String helpers shared by validators and the search index. Kept free of
 * domain imports so it can sit at the bottom of the dependency graph.
 */

export function isBlank(value: string): boolean {
  return value.trim().length === 0;
}

export function truncate(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, Math.max(0, max - 1))}…`;
}

export function slugify(value: string): string {
  return value
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** Case-insensitive substring match used by the task search filter. */
export function containsFold(haystack: string, needle: string): boolean {
  if (needle.length === 0) return true;
  return haystack.toLowerCase().includes(needle.toLowerCase());
}

/** Split a comma-separated tag string into a normalized, deduped list. */
export function parseTags(raw: string): string[] {
  const seen = new Set<string>();
  for (const part of raw.split(",")) {
    const slug = slugify(part);
    if (slug.length > 0) seen.add(slug);
  }
  return [...seen];
}
