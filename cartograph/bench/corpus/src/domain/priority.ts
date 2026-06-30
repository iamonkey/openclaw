/**
 * priority.ts
 *
 * Priority ordering and comparison helpers. The repository uses these to
 * sort task lists and the validators use the rank table to bound input.
 */

import type { Task, TaskPriority } from "./task";

/** Numeric rank: higher is more urgent. */
export const PRIORITY_RANK: Record<TaskPriority, number> = {
  low: 0,
  medium: 1,
  high: 2,
  urgent: 3,
};

export const ALL_PRIORITIES: TaskPriority[] = ["low", "medium", "high", "urgent"];

export function isPriority(value: unknown): value is TaskPriority {
  return typeof value === "string" && value in PRIORITY_RANK;
}

/** Compare two priorities; positive if `a` is more urgent than `b`. */
export function comparePriority(a: TaskPriority, b: TaskPriority): number {
  return PRIORITY_RANK[a] - PRIORITY_RANK[b];
}

/** Sort tasks most-urgent-first, breaking ties by creation time. */
export function byPriorityDesc(a: Task, b: Task): number {
  const delta = comparePriority(b.priority, a.priority);
  if (delta !== 0) return delta;
  return a.createdAt.localeCompare(b.createdAt);
}
