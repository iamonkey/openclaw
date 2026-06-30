/**
 * stats_handler.ts
 *
 * Read-only stats endpoint. Pulls every task from the repository (via a
 * wide list request) and runs the domain stats aggregator. Demonstrates a
 * handler that crosses into the domain/stats module.
 */

import type { Logger } from "../util/logger";
import { computeStats, summarize } from "../domain/stats";
import type { TaskRepository } from "../repo/types";
import { okResponse } from "./respond";
import type { Handler, HttpRequest, HttpResponse } from "./types";

/** Build the stats handler bound to a repository. */
export function makeStatsHandler(repo: TaskRepository, log: Logger): Handler {
  return function statsHandler(_req: HttpRequest): HttpResponse {
    // Pull a maximal page so stats cover the whole store.
    const page = repo.list({}, { page: 1, pageSize: 100 });
    const stats = computeStats(page.items);
    log.info("stats computed", { summary: summarize(stats) });
    return okResponse(stats);
  };
}
