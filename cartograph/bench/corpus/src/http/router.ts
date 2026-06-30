/**
 * router.ts
 *
 * A minimal pattern router. Routes are registered as (method, pattern)
 * pairs where pattern segments beginning with ":" are path params. The
 * router matches a request, fills req.params, and dispatches to the
 * appropriate handler from handlers.ts.
 */

import type { Logger } from "../util/logger";
import { badRequest } from "./respond";
import type { Handler, HttpMethod, HttpRequest, HttpResponse } from "./types";

interface Route {
  method: HttpMethod;
  segments: string[];
  handler: Handler;
}

export class Router {
  private readonly routes: Route[] = [];

  constructor(private readonly log: Logger) {}

  register(method: HttpMethod, pattern: string, handler: Handler): this {
    this.routes.push({ method, segments: splitPath(pattern), handler });
    return this;
  }

  /** Match and dispatch; returns 404 envelope if nothing matches. */
  handle(req: HttpRequest): HttpResponse {
    const reqSegments = splitPath(req.path);
    for (const route of this.routes) {
      if (route.method !== req.method) continue;
      const params = matchSegments(route.segments, reqSegments);
      if (params === null) continue;
      req.params = params;
      this.log.debug("route matched", { method: req.method, path: req.path });
      return route.handler(req);
    }
    return notFoundResponse(req);
  }
}

function splitPath(path: string): string[] {
  return path.split("/").filter((s) => s.length > 0);
}

/**
 * Match route segments against request segments. Returns extracted params on
 * match, or null on mismatch. Lengths must be equal for a match.
 */
function matchSegments(route: string[], actual: string[]): Record<string, string> | null {
  if (route.length !== actual.length) return null;
  const params: Record<string, string> = {};
  for (let i = 0; i < route.length; i += 1) {
    const r = route[i];
    if (r.startsWith(":")) {
      params[r.slice(1)] = actual[i];
    } else if (r !== actual[i]) {
      return null;
    }
  }
  return params;
}

function notFoundResponse(req: HttpRequest): HttpResponse {
  return badRequest(`no route for ${req.method} ${req.path}`);
}

/** Wire all task routes onto a router given a handler set. */
export function buildRouter(handlers: Record<string, Handler>, log: Logger): Router {
  return new Router(log)
    .register("GET", "/health", handlers.health)
    .register("GET", "/tasks", handlers.listTasks)
    .register("POST", "/tasks", handlers.createTask)
    .register("GET", "/tasks/:id", handlers.getTask)
    .register("PATCH", "/tasks/:id", handlers.patchTask)
    .register("POST", "/tasks/:id/status", handlers.transitionTask)
    .register("DELETE", "/tasks/:id", handlers.deleteTask);
}
