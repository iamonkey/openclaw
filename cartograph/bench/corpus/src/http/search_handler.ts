/**
 * search_handler.ts
 *
 * Full-text search endpoint. Reads the `q` query param, delegates to the
 * TaskService search, and returns scored hits. Honors the same structured
 * filters (status/priority/tag) as the list endpoint via parseQuery.
 */

import type { Logger } from "../util/logger";
import type { TaskService } from "../repo/service";
import { parseQuery } from "./parse";
import { badRequest, okResponse } from "./respond";
import type { Handler, HttpRequest, HttpResponse } from "./types";

/** Build the search handler bound to a TaskService. */
export function makeSearchHandler(service: TaskService, log: Logger): Handler {
  return function searchHandler(req: HttpRequest): HttpResponse {
    const text = req.query.q ?? "";
    if (text.trim().length === 0) {
      return badRequest("missing search query 'q'");
    }
    const query = parseQuery(req);
    const hits = service.search(text, query);
    log.debug("search", { q: text, hits: hits.length });
    return okResponse({
      query: text,
      hits: hits.map((h) => ({ id: h.task.id, title: h.task.title, score: h.score })),
    });
  };
}
