/**
 * validate.test.ts
 *
 * Tests for the task input validators. These exercise the cross-file edges
 * validate.ts -> errors.ts, priority.ts, util/strings.ts.
 */

import { describe, expect, it } from "vitest";
import { isErr, isOk } from "../util/result";
import { validateNewTask, validateTaskPatch } from "./validate";

describe("validateNewTask", () => {
  it("rejects a missing title", () => {
    const r = validateNewTask({ description: "no title" });
    expect(isErr(r)).toBe(true);
    if (isErr(r)) {
      expect(r.error.code).toBe("validation");
      expect(r.error.fields?.title).toBeDefined();
    }
  });

  it("accepts a minimal valid payload", () => {
    const r = validateNewTask({ title: "Do the thing" });
    expect(isOk(r)).toBe(true);
    if (isOk(r)) {
      expect(r.value.priority).toBe("medium");
      expect(r.value.tags).toEqual([]);
    }
  });

  it("parses comma-separated tags", () => {
    const r = validateNewTask({ title: "Tagged", tags: "Infra, CI, infra" });
    expect(isOk(r)).toBe(true);
    if (isOk(r)) {
      expect(r.value.tags).toEqual(["infra", "ci"]);
    }
  });

  it("rejects an unknown priority", () => {
    const r = validateNewTask({ title: "x", priority: "critical" });
    expect(isErr(r)).toBe(true);
  });
});

describe("validateTaskPatch", () => {
  it("ignores omitted fields", () => {
    const r = validateTaskPatch({ priority: "high" });
    expect(isOk(r)).toBe(true);
    if (isOk(r)) {
      expect(r.value.title).toBeUndefined();
      expect(r.value.priority).toBe("high");
    }
  });
});
