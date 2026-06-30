/**
 * memory.ts
 *
 * In-memory TaskRepository. Backed by a Map keyed by task id. Filtering and
 * sorting are delegated to the domain query module; pagination is delegated
 * to the util/paginate module. This is the central hub the HTTP handlers
 * talk to, so it imports widely across domain + util.
 */

import { err, isErr, ok, type Result } from "../util/result";
import type { Clock } from "../util/clock";
import { paginate, type Page, type PageRequest } from "../util/paginate";
import {
  type DomainError,
  conflict,
  forbiddenTransition,
  notFound,
} from "../domain/errors";
import { applyQuery, type TaskQuery } from "../domain/query";
import {
  canTransition,
  createTask,
  patchTask,
  withStatus,
  type NewTaskInput,
  type Task,
  type TaskStatus,
} from "../domain/task";
import type { TaskRepository } from "./types";

export class InMemoryTaskRepository implements TaskRepository {
  private readonly store = new Map<string, Task>();

  constructor(private readonly clock: Clock) {}

  create(input: NewTaskInput): Result<Task, DomainError> {
    const task = createTask(input, this.clock);
    if (this.store.has(task.id)) {
      return err(conflict(`id collision: ${task.id}`));
    }
    this.store.set(task.id, task);
    return ok(task);
  }

  get(id: string): Result<Task, DomainError> {
    const task = this.store.get(id);
    if (!task) return err(notFound(`task not found: ${id}`));
    return ok(task);
  }

  list(query: TaskQuery, page: PageRequest): Page<Task> {
    const all = applyQuery([...this.store.values()], query);
    return paginate(all, page);
  }

  patch(id: string, patch: Partial<NewTaskInput>): Result<Task, DomainError> {
    const existing = this.get(id);
    if (isErr(existing)) return existing;
    const updated = patchTask(existing.value, patch, this.clock);
    this.store.set(id, updated);
    return ok(updated);
  }

  transition(id: string, to: TaskStatus): Result<Task, DomainError> {
    const existing = this.get(id);
    if (isErr(existing)) return existing;
    const from = existing.value.status;
    if (!canTransition(from, to)) {
      return err(forbiddenTransition(`cannot move ${from} -> ${to}`));
    }
    const updated = withStatus(existing.value, to, this.clock);
    this.store.set(id, updated);
    return ok(updated);
  }

  remove(id: string): Result<Task, DomainError> {
    const existing = this.get(id);
    if (isErr(existing)) return existing;
    this.store.delete(id);
    return ok(existing.value);
  }

  count(): number {
    return this.store.size;
  }

  /** Test/seed helper: bulk insert pre-built tasks. */
  seed(tasks: Task[]): void {
    for (const t of tasks) this.store.set(t.id, t);
  }
}
