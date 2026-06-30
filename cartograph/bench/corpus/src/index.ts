/**
 * index.ts
 *
 * Public barrel for the task-management library. Re-exports the composition
 * root and the most useful domain/util types so embedders can import from a
 * single entry point.
 */

export { createApp, type App } from "./server";
export { main } from "./main";

export type { Task, TaskStatus, TaskPriority, NewTaskInput } from "./domain/task";
export type { TaskQuery, SortKey } from "./domain/query";
export type { TaskStats } from "./domain/stats";
export type { DomainError, DomainErrorCode } from "./domain/errors";

export { InMemoryTaskRepository } from "./repo/memory";
export { ObservableTaskRepository } from "./repo/observable";
export { AuditTrail } from "./repo/audit";
export { TaskService } from "./repo/service";
export { seededRepository, buildSeedTasks } from "./repo/seed";

export { EventBus, type TaskEvent } from "./domain/events";
export { SearchIndex, type SearchHit } from "./domain/search";
export { computeStats } from "./domain/stats";

export type { Page, PageRequest } from "./util/paginate";
export { paginate, normalizePageRequest } from "./util/paginate";
export { type Result, ok, err, isOk, isErr } from "./util/result";

export { loadConfig } from "./config/load";
export { DEFAULT_CONFIG, type AppConfig } from "./config/schema";
