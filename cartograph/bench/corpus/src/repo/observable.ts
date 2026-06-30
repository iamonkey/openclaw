/**
 * observable.ts
 *
 * A TaskRepository decorator that emits domain events on every mutation by
 * wrapping an inner repository and publishing to an EventBus. The AuditTrail
 * subscribes to the same bus. This keeps the core InMemoryTaskRepository free
 * of event concerns while still giving the rest of the system a change feed.
 */

import { isOk, type Result } from "../util/result";
import type { Page, PageRequest } from "../util/paginate";
import type { DomainError } from "../domain/errors";
import type { EventBus } from "../domain/events";
import type { TaskQuery } from "../domain/query";
import type { NewTaskInput, Task, TaskStatus } from "../domain/task";
import type { TaskRepository } from "./types";

/**
 * Wrap a repository so that successful mutations publish events. Read methods
 * pass straight through. The decorator implements TaskRepository so it is a
 * drop-in replacement at the composition root.
 */
export class ObservableTaskRepository implements TaskRepository {
  constructor(
    private readonly inner: TaskRepository,
    private readonly bus: EventBus,
  ) {}

  create(input: NewTaskInput): Result<Task, DomainError> {
    const result = this.inner.create(input);
    if (isOk(result)) this.bus.emitCreated(result.value);
    return result;
  }

  get(id: string): Result<Task, DomainError> {
    return this.inner.get(id);
  }

  list(query: TaskQuery, page: PageRequest): Page<Task> {
    return this.inner.list(query, page);
  }

  patch(id: string, patch: Partial<NewTaskInput>): Result<Task, DomainError> {
    const result = this.inner.patch(id, patch);
    if (isOk(result)) this.bus.emitUpdated(result.value);
    return result;
  }

  transition(id: string, to: TaskStatus): Result<Task, DomainError> {
    // Capture the prior status so the event can carry the from->to pair.
    const before = this.inner.get(id);
    const result = this.inner.transition(id, to);
    if (isOk(result) && isOk(before)) {
      this.bus.emitTransitioned(result.value, before.value.status, to);
    }
    return result;
  }

  remove(id: string): Result<Task, DomainError> {
    const result = this.inner.remove(id);
    if (isOk(result)) this.bus.emitDeleted(result.value.id);
    return result;
  }

  count(): number {
    return this.inner.count();
  }
}
