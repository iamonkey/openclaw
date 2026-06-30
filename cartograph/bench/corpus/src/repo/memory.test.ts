/**
 * memory.test.ts
 *
 * Tests for the in-memory repository, covering create/get/transition and the
 * lifecycle state machine guard. Exercises repo -> domain -> util edges.
 */

import { describe, expect, it } from "vitest";
import { fixedClock } from "../util/clock";
import { resetIds } from "../util/ids";
import { isErr, isOk } from "../util/result";
import { InMemoryTaskRepository } from "./memory";

function freshRepo(): InMemoryTaskRepository {
  resetIds();
  return new InMemoryTaskRepository(fixedClock(1_700_000_000_000));
}

describe("InMemoryTaskRepository", () => {
  it("creates and reads back a task", () => {
    const repo = freshRepo();
    const created = repo.create({ title: "alpha" });
    expect(isOk(created)).toBe(true);
    if (isOk(created)) {
      const got = repo.get(created.value.id);
      expect(isOk(got)).toBe(true);
    }
  });

  it("404s on missing id", () => {
    const repo = freshRepo();
    const got = repo.get("tsk_missing");
    expect(isErr(got)).toBe(true);
    if (isErr(got)) expect(got.error.code).toBe("not_found");
  });

  it("forbids an illegal transition", () => {
    const repo = freshRepo();
    const created = repo.create({ title: "beta" });
    if (isOk(created)) {
      const done = repo.transition(created.value.id, "done");
      expect(isOk(done)).toBe(true);
      const back = repo.transition(created.value.id, "blocked");
      expect(isErr(back)).toBe(true);
      if (isErr(back)) expect(back.error.code).toBe("forbidden_transition");
    }
  });

  it("lists with a status filter", () => {
    const repo = freshRepo();
    repo.create({ title: "one" });
    repo.create({ title: "two" });
    const page = repo.list({ status: "open" }, { page: 1, pageSize: 10 });
    expect(page.totalItems).toBe(2);
  });
});
