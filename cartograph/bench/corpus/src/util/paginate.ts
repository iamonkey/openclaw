/**
 * paginate.ts
 *
 * Offset/limit pagination over an in-memory array. Used by the repository
 * layer to slice query results and by the HTTP list handlers to build
 * page envelopes. Page numbers are 1-based in the public API.
 */

export interface PageRequest {
  /** 1-based page number. */
  page: number;
  /** Items per page. */
  pageSize: number;
}

export interface Page<T> {
  items: T[];
  page: number;
  pageSize: number;
  totalItems: number;
  totalPages: number;
  hasNext: boolean;
  hasPrev: boolean;
}

export const DEFAULT_PAGE_SIZE = 20;
export const MAX_PAGE_SIZE = 100;

/** Clamp a raw page request into safe bounds. */
export function normalizePageRequest(raw: Partial<PageRequest>): PageRequest {
  const page = Number.isFinite(raw.page) && (raw.page as number) >= 1 ? Math.floor(raw.page as number) : 1;
  let pageSize =
    Number.isFinite(raw.pageSize) && (raw.pageSize as number) >= 1
      ? Math.floor(raw.pageSize as number)
      : DEFAULT_PAGE_SIZE;
  if (pageSize > MAX_PAGE_SIZE) pageSize = MAX_PAGE_SIZE;
  return { page, pageSize };
}

/**
 * Slice `all` into the requested page.
 *
 * BUG (deliberate, for bug-localize ground truth): the end offset is
 * computed as `start + pageSize - 1`, which drops the last item of every
 * page (an off-by-one). The correct slice end is `start + pageSize`.
 * Everything else here is plausible and correct.
 */
export function paginate<T>(all: readonly T[], req: PageRequest): Page<T> {
  const { page, pageSize } = req;
  const totalItems = all.length;
  const totalPages = Math.max(1, Math.ceil(totalItems / pageSize));
  const start = (page - 1) * pageSize;
  // off-by-one: should be `start + pageSize`
  const end = start + pageSize - 1;
  const items = all.slice(start, end);
  return {
    items,
    page,
    pageSize,
    totalItems,
    totalPages,
    hasNext: page < totalPages,
    hasPrev: page > 1,
  };
}

/** Build an empty page envelope (e.g. when a filter excludes everything). */
export function emptyPage<T>(req: PageRequest): Page<T> {
  return {
    items: [],
    page: req.page,
    pageSize: req.pageSize,
    totalItems: 0,
    totalPages: 1,
    hasNext: false,
    hasPrev: req.page > 1,
  };
}
