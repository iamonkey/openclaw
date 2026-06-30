/**
 * task.ts
 *
 * Core domain entity: the Task. Defines the Task shape, its lifecycle
 * status enum, and pure factory/transition helpers. The repository stores
 * Task records and the HTTP layer serializes them.
 */

import type { Clock } from "../util/clock";
import { nextId } from "../util/ids";

export type TaskStatus = "open" | "in_progress" | "blocked" | "done";

export type TaskPriority = "low" | "medium" | "high" | "urgent";

export interface Task {
  id: string;
  title: string;
  description: string;
  status: TaskStatus;
  priority: TaskPriority;
  tags: string[];
  assignee: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface NewTaskInput {
  title: string;
  description?: string;
  priority?: TaskPriority;
  tags?: string[];
  assignee?: string | null;
}

const VALID_TRANSITIONS: Record<TaskStatus, TaskStatus[]> = {
  open: ["in_progress", "blocked", "done"],
  in_progress: ["blocked", "done", "open"],
  blocked: ["in_progress", "open"],
  done: ["open"],
};

/** Create a fresh Task from validated input. */
export function createTask(input: NewTaskInput, clock: Clock): Task {
  const ts = clock.iso();
  return {
    id: nextId(),
    title: input.title,
    description: input.description ?? "",
    status: "open",
    priority: input.priority ?? "medium",
    tags: input.tags ?? [],
    assignee: input.assignee ?? null,
    createdAt: ts,
    updatedAt: ts,
  };
}

/** Whether a status change is allowed by the lifecycle state machine. */
export function canTransition(from: TaskStatus, to: TaskStatus): boolean {
  if (from === to) return true;
  return VALID_TRANSITIONS[from].includes(to);
}

/** Apply a status transition, returning a new Task (immutably). */
export function withStatus(task: Task, status: TaskStatus, clock: Clock): Task {
  return { ...task, status, updatedAt: clock.iso() };
}

/** Merge a partial patch onto a task, bumping updatedAt. */
export function patchTask(task: Task, patch: Partial<NewTaskInput>, clock: Clock): Task {
  return {
    ...task,
    title: patch.title ?? task.title,
    description: patch.description ?? task.description,
    priority: patch.priority ?? task.priority,
    tags: patch.tags ?? task.tags,
    assignee: patch.assignee === undefined ? task.assignee : patch.assignee,
    updatedAt: clock.iso(),
  };
}

export function isTerminal(task: Task): boolean {
  return task.status === "done";
}
