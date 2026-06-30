/**
 * search.test.ts
 *
 * Tests for the inverted-index search. Exercises tokenization, stop-word
 * filtering, and score-ordered results.
 */

import { describe, expect, it } from "vitest";
import { fixedClock } from "../util/clock";
import { resetIds } from "../util/ids";
import { createTask } from "./task";
import { SearchIndex, tokenize } from "./search";

const clock = fixedClock(1_700_000_000_000);

function build(titles: string[]) {
  resetIds();
  return titles.map((title) => createTask({ title }, clock));
}

describe("tokenize", () => {
  it("drops stop words and normalizes", () => {
    expect(tokenize("Set up the CI Pipeline")).toEqual(["set", "up", "ci", "pipeline"]);
  });
});

describe("SearchIndex", () => {
  it("returns tasks matching a query term", () => {
    const tasks = build(["Write README docs", "Set up CI", "Design pagination"]);
    const index = new SearchIndex();
    index.indexAll(tasks);
    const hits = index.search("pagination");
    expect(hits.length).toBe(1);
    expect(hits[0].task.title).toContain("pagination");
  });

  it("ranks multi-term matches higher", () => {
    const tasks = build(["pagination api design", "design only", "api only"]);
    const index = new SearchIndex();
    index.indexAll(tasks);
    const hits = index.search("api design");
    expect(hits[0].score).toBeGreaterThanOrEqual(hits[hits.length - 1].score);
  });

  it("removes a task from the index", () => {
    const tasks = build(["alpha", "beta"]);
    const index = new SearchIndex();
    index.indexAll(tasks);
    index.remove(tasks[0].id);
    expect(index.search("alpha").length).toBe(0);
    expect(index.size()).toBe(1);
  });
});
