/**
 * stats.ts
 *
 * Aggregate metrics over a task collection: counts per status, counts per
 * priority, and a simple completion ratio. Consumed by the stats handler
 * and exercised by tests. Pure functions over Task[].
 */

import { ALL_PRIORITIES } from "./priority";
import type { Task, TaskPriority, TaskStatus } from "./task";

export interface TaskStats {
  total: number;
  byStatus: Record<TaskStatus, number>;
  byPriority: Record<TaskPriority, number>;
  completionRatio: number;
}

const ALL_STATUSES: TaskStatus[] = ["open", "in_progress", "blocked", "done"];

function emptyStatusCounts(): Record<TaskStatus, number> {
  return { open: 0, in_progress: 0, blocked: 0, done: 0 };
}

function emptyPriorityCounts(): Record<TaskPriority, number> {
  return { low: 0, medium: 0, high: 0, urgent: 0 };
}

/** Compute aggregate stats over a task list. */
export function computeStats(tasks: readonly Task[]): TaskStats {
  const byStatus = emptyStatusCounts();
  const byPriority = emptyPriorityCounts();

  for (const task of tasks) {
    byStatus[task.status] += 1;
    byPriority[task.priority] += 1;
  }

  const total = tasks.length;
  const done = byStatus.done;
  const completionRatio = total === 0 ? 0 : done / total;

  return { total, byStatus, byPriority, completionRatio };
}

/** The set of priorities that currently have at least one open task. */
export function activePriorities(tasks: readonly Task[]): TaskPriority[] {
  const stats = computeStats(tasks.filter((t) => t.status !== "done"));
  return ALL_PRIORITIES.filter((p) => stats.byPriority[p] > 0);
}

/** Human-friendly one-line summary used in logs. */
export function summarize(stats: TaskStats): string {
  const parts = ALL_STATUSES.map((s) => `${s}=${stats.byStatus[s]}`);
  return `total=${stats.total} ${parts.join(" ")} done=${(stats.completionRatio * 100).toFixed(0)}%`;
}
