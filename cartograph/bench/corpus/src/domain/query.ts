/**
 * query.ts
 *
 * Query/filter descriptors for listing tasks. The repository interprets a
 * TaskQuery to filter and sort its in-memory store before pagination.
 */

import { containsFold } from "../util/strings";
import { byPriorityDesc } from "./priority";
import type { Task, TaskPriority, TaskStatus } from "./task";

export type SortKey = "created" | "updated" | "priority";

export interface TaskQuery {
  status?: TaskStatus;
  priority?: TaskPriority;
  assignee?: string;
  tag?: string;
  search?: string;
  sort?: SortKey;
}

/** Return true if a task satisfies every set filter in the query. */
export function matchesQuery(task: Task, query: TaskQuery): boolean {
  if (query.status && task.status !== query.status) return false;
  if (query.priority && task.priority !== query.priority) return false;
  if (query.assignee && task.assignee !== query.assignee) return false;
  if (query.tag && !task.tags.includes(query.tag)) return false;
  if (query.search) {
    const inTitle = containsFold(task.title, query.search);
    const inDesc = containsFold(task.description, query.search);
    if (!inTitle && !inDesc) return false;
  }
  return true;
}

function byCreatedDesc(a: Task, b: Task): number {
  return b.createdAt.localeCompare(a.createdAt);
}

function byUpdatedDesc(a: Task, b: Task): number {
  return b.updatedAt.localeCompare(a.updatedAt);
}

/** Pick the comparator for a sort key. Defaults to created-desc. */
export function comparatorFor(sort: SortKey | undefined): (a: Task, b: Task) => number {
  switch (sort) {
    case "priority":
      return byPriorityDesc;
    case "updated":
      return byUpdatedDesc;
    case "created":
    default:
      return byCreatedDesc;
  }
}

/** Apply filter + sort to a task list (pagination happens after). */
export function applyQuery(tasks: readonly Task[], query: TaskQuery): Task[] {
  const filtered = tasks.filter((t) => matchesQuery(t, query));
  filtered.sort(comparatorFor(query.sort));
  return filtered;
}
