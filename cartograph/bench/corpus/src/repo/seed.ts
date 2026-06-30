/**
 * seed.ts
 *
 * Deterministic seed data for local dev and tests. Builds a handful of
 * tasks using a fixed clock so ids/timestamps are reproducible.
 */

import { fixedClock } from "../util/clock";
import { resetIds } from "../util/ids";
import { createTask, type Task } from "../domain/task";
import { InMemoryTaskRepository } from "./memory";

const SEED_EPOCH = 1_700_000_000_000;

const SEED_INPUTS = [
  { title: "Write project README", priority: "high" as const, tags: ["docs"] },
  { title: "Set up CI pipeline", priority: "urgent" as const, tags: ["infra", "ci"] },
  { title: "Design pagination API", priority: "medium" as const, tags: ["api"] },
  { title: "Add input validation", priority: "high" as const, tags: ["api", "safety"] },
  { title: "Triage open issues", priority: "low" as const, tags: ["triage"] },
];

/** Build the canonical seed task list. */
export function buildSeedTasks(): Task[] {
  resetIds();
  const clock = fixedClock(SEED_EPOCH);
  return SEED_INPUTS.map((input) => createTask(input, clock));
}

/** Construct an in-memory repository pre-loaded with seed data. */
export function seededRepository(): InMemoryTaskRepository {
  const clock = fixedClock(SEED_EPOCH);
  const repo = new InMemoryTaskRepository(clock);
  repo.seed(buildSeedTasks());
  return repo;
}
