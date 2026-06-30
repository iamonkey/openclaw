/**
 * main.ts
 *
 * CLI entry point. Boots the app from process.env, fires a couple of
 * synthetic requests through the router to demonstrate the request flow,
 * and prints the responses. Not a real server loop — the corpus models the
 * shape of one without binding a socket.
 */

import { isErr } from "./util/result";
import { createApp } from "./server";
import type { HttpRequest } from "./http/types";

function request(
  method: HttpRequest["method"],
  path: string,
  body?: unknown,
  query: Record<string, string> = {},
): HttpRequest {
  return { method, path, params: {}, query, body };
}

export function main(env: Record<string, string | undefined> = process.env): number {
  const appResult = createApp(env, { seed: true });
  if (isErr(appResult)) {
    console.error(`failed to start: ${appResult.error}`);
    return 1;
  }
  const { router, log } = appResult.value;

  const health = router.handle(request("GET", "/health"));
  log.info("health", { status: health.status });

  const created = router.handle(
    request("POST", "/tasks", { title: "Ship Cartograph", priority: "urgent" }),
  );
  log.info("created", { status: created.status });

  const listed = router.handle(request("GET", "/tasks", undefined, { sort: "priority", page: "1" }));
  log.info("listed", { status: listed.status });

  return 0;
}

// Execute when run directly.
if (typeof process !== "undefined" && process.argv[1]?.endsWith("main.ts")) {
  process.exitCode = main();
}
