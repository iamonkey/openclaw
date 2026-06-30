/**
 * server.ts
 *
 * Application bootstrap. Wires config -> logger -> clock -> event bus ->
 * repository (+ observable decorator) -> audit -> service -> handlers ->
 * router into a single object. This is the composition root: it imports from
 * every layer and is the natural starting point for "how does a request flow"
 * questions.
 */

import { isErr, type Result } from "./util/result";
import { Logger } from "./util/logger";
import { systemClock } from "./util/clock";
import { loadConfig } from "./config/load";
import type { AppConfig } from "./config/schema";
import type { EnvBag } from "./config/env";
import { EventBus } from "./domain/events";
import { InMemoryTaskRepository } from "./repo/memory";
import { ObservableTaskRepository } from "./repo/observable";
import { AuditTrail } from "./repo/audit";
import { TaskService } from "./repo/service";
import { buildSeedTasks } from "./repo/seed";
import { makeTaskHandlers } from "./http/handlers";
import { makeStatsHandler } from "./http/stats_handler";
import { makeSearchHandler } from "./http/search_handler";
import { buildRouter, Router } from "./http/router";

export interface App {
  config: AppConfig;
  router: Router;
  repo: ObservableTaskRepository;
  service: TaskService;
  audit: AuditTrail;
  bus: EventBus;
  log: Logger;
}

/** Build the full application graph from an environment bag. */
export function createApp(env: EnvBag, opts?: { seed?: boolean }): Result<App> {
  const configResult = loadConfig(env);
  if (isErr(configResult)) return configResult;
  const config = configResult.value;

  const log = new Logger(config.logLevel);
  const clock = systemClock;
  const bus = new EventBus(clock);

  const inner = new InMemoryTaskRepository(clock);
  if (opts?.seed) {
    inner.seed(buildSeedTasks());
    log.info("seeded repository", { count: inner.count() });
  }

  // The audit trail listens to every mutation via the bus.
  const audit = new AuditTrail().attach(bus);
  const repo = new ObservableTaskRepository(inner, bus);

  const service = new TaskService({ repo, audit });
  service.reindex();

  const handlers = makeTaskHandlers(repo, log);
  handlers.stats = makeStatsHandler(repo, log);
  if (config.features.enableSearch) {
    handlers.search = makeSearchHandler(service, log);
  }

  const router = buildRouter(handlers, log);
  if (config.features.enableSearch) {
    router.register("GET", "/search", handlers.search);
  }
  router.register("GET", "/stats", handlers.stats);

  log.info("app ready", { env: config.env, port: config.server.port });
  return { ok: true, value: { config, router, repo, service, audit, bus, log } };
}
