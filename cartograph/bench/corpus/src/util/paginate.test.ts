/**
 * paginate.test.ts
 *
 * Tests for the pagination utility. NOTE: the `paginate` implementation has a
 * deliberate off-by-one bug (it drops the last item of each page); these
 * tests describe the INTENDED behavior, so they would fail against the
 * current implementation. They serve as the ground-truth spec for the
 * bug-localize benchmark task.
 */

import { describe, expect, it } from "vitest";
import { normalizePageRequest, paginate } from "./paginate";

const data = Array.from({ length: 25 }, (_, i) => i + 1);

describe("paginate", () => {
  it("returns a full first page", () => {
    const page = paginate(data, { page: 1, pageSize: 10 });
    // Intended: 10 items [1..10]. Bug yields 9.
    expect(page.items.length).toBe(10);
    expect(page.items[0]).toBe(1);
    expect(page.items[9]).toBe(10);
  });

  it("computes total pages", () => {
    const page = paginate(data, { page: 1, pageSize: 10 });
    expect(page.totalPages).toBe(3);
    expect(page.hasNext).toBe(true);
    expect(page.hasPrev).toBe(false);
  });

  it("handles the last partial page", () => {
    const page = paginate(data, { page: 3, pageSize: 10 });
    // Intended: items [21..25].
    expect(page.items).toEqual([21, 22, 23, 24, 25]);
    expect(page.hasNext).toBe(false);
  });
});

describe("normalizePageRequest", () => {
  it("clamps page size to the max", () => {
    const req = normalizePageRequest({ page: 1, pageSize: 9999 });
    expect(req.pageSize).toBe(100);
  });

  it("defaults invalid input", () => {
    const req = normalizePageRequest({ page: -3, pageSize: 0 });
    expect(req.page).toBe(1);
    expect(req.pageSize).toBe(20);
  });
});
