/**
 * handlers.ts
 *
 * Task CRUD handlers. Each handler validates input via the domain
 * validators, calls into the TaskRepository, and maps the Result to an
 * HttpResponse. This is the busiest cross-file caller in the codebase:
 * it pulls from domain, repo, util, and the local http helpers.
 */

import { isErr } from "../util/result";
import type { Logger } from "../util/logger";
import { validationError } from "../domain/errors";
import { validateNewTask, validateTaskPatch } from "../domain/validate";
import { canTransition, type TaskStatus } from "../domain/task";
import type { TaskRepository } from "../repo/types";
import { parsePage, parseQuery } from "./parse";
import { badRequest, errorResponse, okResponse } from "./respond";
import type { Handler, HttpRequest, HttpResponse } from "./types";

const TRANSITION_TARGETS: TaskStatus[] = ["open", "in_progress", "blocked", "done"];

/** Build the handler set bound to a repository and logger. */
export function makeTaskHandlers(repo: TaskRepository, log: Logger): Record<string, Handler> {
  function listTasks(req: HttpRequest): HttpResponse {
    const query = parseQuery(req);
    const page = parsePage(req);
    const result = repo.list(query, page);
    log.debug("list tasks", { count: result.items.length, page: page.page });
    return okResponse(result);
  }

  function getTask(req: HttpRequest): HttpResponse {
    const id = req.params.id;
    if (!id) return badRequest("missing id");
    const result = repo.get(id);
    if (isErr(result)) return errorResponse(result.error);
    return okResponse(result.value);
  }

  function createTaskHandler(req: HttpRequest): HttpResponse {
    const validated = validateNewTask(req.body);
    if (isErr(validated)) return errorResponse(validated.error);
    const created = repo.create(validated.value);
    if (isErr(created)) return errorResponse(created.error);
    log.info("task created", { id: created.value.id });
    return okResponse(created.value, 201);
  }

  function patchTaskHandler(req: HttpRequest): HttpResponse {
    const id = req.params.id;
    if (!id) return badRequest("missing id");
    const validated = validateTaskPatch(req.body);
    if (isErr(validated)) return errorResponse(validated.error);
    const updated = repo.patch(id, validated.value);
    if (isErr(updated)) return errorResponse(updated.error);
    return okResponse(updated.value);
  }

  function transitionTaskHandler(req: HttpRequest): HttpResponse {
    const id = req.params.id;
    if (!id) return badRequest("missing id");
    const to = (req.body as { status?: string } | undefined)?.status;
    if (!to || !(TRANSITION_TARGETS as string[]).includes(to)) {
      return errorResponse(validationError("invalid target status", { status: "required" }));
    }
    const current = repo.get(id);
    if (isErr(current)) return errorResponse(current.error);
    if (!canTransition(current.value.status, to as TaskStatus)) {
      log.warn("forbidden transition", { id, from: current.value.status, to });
    }
    const moved = repo.transition(id, to as TaskStatus);
    if (isErr(moved)) return errorResponse(moved.error);
    return okResponse(moved.value);
  }

  function deleteTaskHandler(req: HttpRequest): HttpResponse {
    const id = req.params.id;
    if (!id) return badRequest("missing id");
    const removed = repo.remove(id);
    if (isErr(removed)) return errorResponse(removed.error);
    return okResponse({ deleted: removed.value.id });
  }

  function healthHandler(_req: HttpRequest): HttpResponse {
    return okResponse({ status: "ok", tasks: repo.count() });
  }

  return {
    listTasks,
    getTask,
    createTask: createTaskHandler,
    patchTask: patchTaskHandler,
    transitionTask: transitionTaskHandler,
    deleteTask: deleteTaskHandler,
    health: healthHandler,
  };
}
