/**
 * parse.ts
 *
 * Request parsing helpers shared by the list/handler code: extracting a
 * TaskQuery and a PageRequest from the request query string.
 */

import { normalizePageRequest, type PageRequest } from "../util/paginate";
import { isPriority } from "../domain/priority";
import type { SortKey, TaskQuery } from "../domain/query";
import type { TaskStatus } from "../domain/task";
import type { HttpRequest } from "./types";

const STATUSES: TaskStatus[] = ["open", "in_progress", "blocked", "done"];
const SORTS: SortKey[] = ["created", "updated", "priority"];

function asStatus(v: string | undefined): TaskStatus | undefined {
  return v && (STATUSES as string[]).includes(v) ? (v as TaskStatus) : undefined;
}

function asSort(v: string | undefined): SortKey | undefined {
  return v && (SORTS as string[]).includes(v) ? (v as SortKey) : undefined;
}

/** Build a TaskQuery from the request's query string. */
export function parseQuery(req: HttpRequest): TaskQuery {
  const q = req.query;
  return {
    status: asStatus(q.status),
    priority: isPriority(q.priority) ? q.priority : undefined,
    assignee: q.assignee || undefined,
    tag: q.tag || undefined,
    search: q.search || undefined,
    sort: asSort(q.sort),
  };
}

/** Build a clamped PageRequest from the request's query string. */
export function parsePage(req: HttpRequest): PageRequest {
  return normalizePageRequest({
    page: req.query.page ? Number.parseInt(req.query.page, 10) : undefined,
    pageSize: req.query.pageSize ? Number.parseInt(req.query.pageSize, 10) : undefined,
  });
}
