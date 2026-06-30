/**
 * events.ts
 *
 * Domain event types and a tiny in-memory event sink. The audit module
 * subscribes to these to build a history. Events are emitted as plain
 * tagged objects so they are easy to serialize and assert against.
 */

import type { Clock } from "../util/clock";
import type { Task, TaskStatus } from "./task";

export type TaskEvent =
  | { type: "task.created"; at: string; taskId: string }
  | { type: "task.updated"; at: string; taskId: string }
  | { type: "task.transitioned"; at: string; taskId: string; from: TaskStatus; to: TaskStatus }
  | { type: "task.deleted"; at: string; taskId: string };

export type EventListener = (event: TaskEvent) => void;

export class EventBus {
  private readonly listeners: EventListener[] = [];

  constructor(private readonly clock: Clock) {}

  subscribe(listener: EventListener): () => void {
    this.listeners.push(listener);
    return () => {
      const idx = this.listeners.indexOf(listener);
      if (idx >= 0) this.listeners.splice(idx, 1);
    };
  }

  private publish(event: TaskEvent): void {
    for (const l of this.listeners) l(event);
  }

  emitCreated(task: Task): void {
    this.publish({ type: "task.created", at: this.clock.iso(), taskId: task.id });
  }

  emitUpdated(task: Task): void {
    this.publish({ type: "task.updated", at: this.clock.iso(), taskId: task.id });
  }

  emitTransitioned(task: Task, from: TaskStatus, to: TaskStatus): void {
    this.publish({ type: "task.transitioned", at: this.clock.iso(), taskId: task.id, from, to });
  }

  emitDeleted(taskId: string): void {
    this.publish({ type: "task.deleted", at: this.clock.iso(), taskId });
  }
}
