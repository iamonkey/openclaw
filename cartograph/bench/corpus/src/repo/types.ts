/**
 * types.ts
 *
 * The repository port. The HTTP layer depends on this interface, not on the
 * concrete in-memory implementation, so storage can be swapped later.
 */

import type { Result } from "../util/result";
import type { DomainError } from "../domain/errors";
import type { Page, PageRequest } from "../util/paginate";
import type { TaskQuery } from "../domain/query";
import type { NewTaskInput, Task, TaskStatus } from "../domain/task";

export interface TaskRepository {
  create(input: NewTaskInput): Result<Task, DomainError>;
  get(id: string): Result<Task, DomainError>;
  list(query: TaskQuery, page: PageRequest): Page<Task>;
  patch(id: string, patch: Partial<NewTaskInput>): Result<Task, DomainError>;
  transition(id: string, to: TaskStatus): Result<Task, DomainError>;
  remove(id: string): Result<Task, DomainError>;
  count(): number;
}
