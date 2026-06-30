/**
 * audit.ts
 *
 * An audit trail built by subscribing to the EventBus. Stores a bounded
 * ring of recent events and exposes a paginated read. Demonstrates a second
 * consumer of util/paginate and of the domain event types.
 */

import { paginate, type Page, type PageRequest } from "../util/paginate";
import type { EventBus, TaskEvent } from "../domain/events";

const DEFAULT_CAPACITY = 500;

export class AuditTrail {
  private readonly events: TaskEvent[] = [];
  private unsubscribe: (() => void) | null = null;

  constructor(private readonly capacity: number = DEFAULT_CAPACITY) {}

  /** Begin recording events from a bus. Returns this for chaining. */
  attach(bus: EventBus): this {
    this.unsubscribe = bus.subscribe((event) => this.record(event));
    return this;
  }

  detach(): void {
    if (this.unsubscribe) {
      this.unsubscribe();
      this.unsubscribe = null;
    }
  }

  private record(event: TaskEvent): void {
    this.events.push(event);
    // Trim from the front when over capacity (bounded ring).
    if (this.events.length > this.capacity) {
      this.events.splice(0, this.events.length - this.capacity);
    }
  }

  size(): number {
    return this.events.length;
  }

  /** Read recent events newest-first, paginated. */
  list(page: PageRequest): Page<TaskEvent> {
    const newestFirst = [...this.events].reverse();
    return paginate(newestFirst, page);
  }

  /** All events touching a specific task id, oldest-first. */
  forTask(taskId: string): TaskEvent[] {
    return this.events.filter((e) => e.taskId === taskId);
  }
}
