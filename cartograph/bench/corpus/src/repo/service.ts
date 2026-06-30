/**
 * service.ts
 *
 * The TaskService is a thin application service that composes the repository,
 * the search index, and the audit trail behind a single API the HTTP layer
 * can call. It keeps the search index in sync on writes and exposes a
 * full-text search that falls back to the repository's structured list.
 */

import { isOk, type Result } from "../util/result";
import type { Page, PageRequest } from "../util/paginate";
import type { DomainError } from "../domain/errors";
import { applyQuery, type TaskQuery } from "../domain/query";
import { SearchIndex, type SearchHit } from "../domain/search";
import type { NewTaskInput, Task, TaskStatus } from "../domain/task";
import type { AuditTrail } from "./audit";
import type { TaskRepository } from "./types";

export interface ServiceDeps {
  repo: TaskRepository;
  audit: AuditTrail;
}

export class TaskService {
  private readonly index = new SearchIndex();

  constructor(private readonly deps: ServiceDeps) {}

  /** Rebuild the search index from the current repository contents. */
  reindex(): void {
    const page = this.deps.repo.list({}, { page: 1, pageSize: 100 });
    this.index.indexAll(page.items);
  }

  create(input: NewTaskInput): Result<Task, DomainError> {
    const result = this.deps.repo.create(input);
    if (isOk(result)) this.index.index(result.value);
    return result;
  }

  patch(id: string, patch: Partial<NewTaskInput>): Result<Task, DomainError> {
    const result = this.deps.repo.patch(id, patch);
    if (isOk(result)) this.index.index(result.value);
    return result;
  }

  transition(id: string, to: TaskStatus): Result<Task, DomainError> {
    const result = this.deps.repo.transition(id, to);
    if (isOk(result)) this.index.index(result.value);
    return result;
  }

  remove(id: string): Result<Task, DomainError> {
    const result = this.deps.repo.remove(id);
    if (isOk(result)) this.index.remove(id);
    return result;
  }

  get(id: string): Result<Task, DomainError> {
    return this.deps.repo.get(id);
  }

  list(query: TaskQuery, page: PageRequest): Page<Task> {
    return this.deps.repo.list(query, page);
  }

  /** Full-text search via the inverted index, then re-applies structured
   * filters from the query so search and filters compose. */
  search(text: string, query: TaskQuery): SearchHit[] {
    const hits = this.index.search(text);
    const tasks = hits.map((h) => h.task);
    const allowed = new Set(applyQuery(tasks, query).map((t) => t.id));
    return hits.filter((h) => allowed.has(h.task.id));
  }

  auditFor(id: string) {
    return this.deps.audit.forTask(id);
  }
}
