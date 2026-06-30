/**
 * search.ts
 *
 * A small inverted-index search over task titles, descriptions, and tags.
 * Tokens are normalized via util/strings helpers. The repository can build
 * an index lazily; the HTTP search filter falls back to this when the
 * `enableSearch` feature flag is on and the query is non-trivial.
 */

import { slugify } from "../util/strings";
import type { Task } from "./task";

export interface SearchHit {
  task: Task;
  score: number;
}

const STOP_WORDS = new Set(["the", "a", "an", "to", "of", "and", "for", "in", "on"]);

/** Tokenize free text into normalized, stop-word-filtered terms. */
export function tokenize(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split(/\s+/)) {
    const term = slugify(raw);
    if (term.length === 0) continue;
    if (STOP_WORDS.has(term)) continue;
    out.push(term);
  }
  return out;
}

/** Build the searchable term bag for a single task. */
export function termsForTask(task: Task): string[] {
  return [
    ...tokenize(task.title),
    ...tokenize(task.description),
    ...task.tags.map((t) => slugify(t)),
  ];
}

export class SearchIndex {
  /** term -> set of task ids */
  private readonly postings = new Map<string, Set<string>>();
  private readonly tasksById = new Map<string, Task>();

  /** (Re)index a single task, replacing any prior entry. */
  index(task: Task): void {
    this.remove(task.id);
    this.tasksById.set(task.id, task);
    for (const term of new Set(termsForTask(task))) {
      let bucket = this.postings.get(term);
      if (!bucket) {
        bucket = new Set();
        this.postings.set(term, bucket);
      }
      bucket.add(task.id);
    }
  }

  /** Index a whole collection. */
  indexAll(tasks: readonly Task[]): void {
    for (const t of tasks) this.index(t);
  }

  remove(taskId: string): void {
    this.tasksById.delete(taskId);
    for (const bucket of this.postings.values()) {
      bucket.delete(taskId);
    }
  }

  /**
   * Score documents by the number of distinct query terms they match.
   * Returns hits sorted by descending score then by task id for stability.
   */
  search(queryText: string): SearchHit[] {
    const terms = new Set(tokenize(queryText));
    if (terms.size === 0) return [];

    const scores = new Map<string, number>();
    for (const term of terms) {
      const bucket = this.postings.get(term);
      if (!bucket) continue;
      for (const id of bucket) {
        scores.set(id, (scores.get(id) ?? 0) + 1);
      }
    }

    const hits: SearchHit[] = [];
    for (const [id, score] of scores) {
      const task = this.tasksById.get(id);
      if (task) hits.push({ task, score });
    }
    hits.sort((a, b) => b.score - a.score || a.task.id.localeCompare(b.task.id));
    return hits;
  }

  size(): number {
    return this.tasksById.size;
  }
}
