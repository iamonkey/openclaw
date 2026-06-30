/**
 * router.test.ts
 *
 * End-to-end tests over the assembled app: dispatch real HttpRequests through
 * the router and assert on status codes and bodies. Exercises the full
 * server -> router -> handlers -> service -> repo -> domain -> util chain.
 */

import { describe, expect, it } from "vitest";
import { isOk } from "../util/result";
import { createApp } from "../server";
import type { HttpRequest } from "./types";

function req(
  method: HttpRequest["method"],
  path: string,
  body?: unknown,
  query: Record<string, string> = {},
): HttpRequest {
  return { method, path, params: {}, query, body };
}

function boot() {
  const r = createApp({ LOG_LEVEL: "error" }, { seed: true });
  if (!isOk(r)) throw new Error("app failed to boot");
  return r.value;
}

describe("router", () => {
  it("serves health", () => {
    const app = boot();
    const res = app.router.handle(req("GET", "/health"));
    expect(res.status).toBe(200);
  });

  it("creates then fetches a task", () => {
    const app = boot();
    const created = app.router.handle(req("POST", "/tasks", { title: "from test" }));
    expect(created.status).toBe(201);
    const id = (created.body as { id: string }).id;
    const got = app.router.handle(req("GET", `/tasks/${id}`));
    expect(got.status).toBe(200);
  });

  it("422s on invalid create", () => {
    const app = boot();
    const res = app.router.handle(req("POST", "/tasks", { description: "no title" }));
    expect(res.status).toBe(422);
  });

  it("404s on unknown task", () => {
    const app = boot();
    const res = app.router.handle(req("GET", "/tasks/tsk_nope"));
    expect(res.status).toBe(404);
  });

  it("rejects an unknown route", () => {
    const app = boot();
    const res = app.router.handle(req("GET", "/nope"));
    expect(res.status).toBe(400);
  });
});
