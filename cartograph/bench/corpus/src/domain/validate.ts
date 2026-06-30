/**
 * validate.ts
 *
 * Input validation for task creation and patching. Returns Result with a
 * DomainError on failure. The HTTP handlers call these before touching the
 * repository so invalid payloads never reach storage.
 */

import { err, ok, type Result } from "../util/result";
import { isBlank, parseTags } from "../util/strings";
import { isPriority } from "./priority";
import { type DomainError, validationError } from "./errors";
import type { NewTaskInput, TaskPriority } from "./task";

const TITLE_MAX = 200;
const DESC_MAX = 4000;
const TAGS_MAX = 16;

/** Validate a raw create payload into a clean NewTaskInput. */
export function validateNewTask(raw: unknown): Result<NewTaskInput, DomainError> {
  if (typeof raw !== "object" || raw === null) {
    return err(validationError("body must be an object"));
  }
  const body = raw as Record<string, unknown>;
  const fields: Record<string, string> = {};

  const title = typeof body.title === "string" ? body.title.trim() : "";
  if (isBlank(title)) {
    fields.title = "title is required";
  } else if (title.length > TITLE_MAX) {
    fields.title = `title exceeds ${TITLE_MAX} chars`;
  }

  let description = "";
  if (body.description !== undefined) {
    if (typeof body.description !== "string") {
      fields.description = "description must be a string";
    } else if (body.description.length > DESC_MAX) {
      fields.description = `description exceeds ${DESC_MAX} chars`;
    } else {
      description = body.description;
    }
  }

  let priority: TaskPriority = "medium";
  if (body.priority !== undefined) {
    if (!isPriority(body.priority)) {
      fields.priority = "priority must be one of low|medium|high|urgent";
    } else {
      priority = body.priority;
    }
  }

  let tags: string[] = [];
  if (body.tags !== undefined) {
    if (typeof body.tags === "string") {
      tags = parseTags(body.tags);
    } else if (Array.isArray(body.tags) && body.tags.every((t) => typeof t === "string")) {
      tags = parseTags((body.tags as string[]).join(","));
    } else {
      fields.tags = "tags must be a string or string[]";
    }
    if (tags.length > TAGS_MAX) {
      fields.tags = `at most ${TAGS_MAX} tags allowed`;
    }
  }

  let assignee: string | null = null;
  if (body.assignee !== undefined && body.assignee !== null) {
    if (typeof body.assignee !== "string" || isBlank(body.assignee)) {
      fields.assignee = "assignee must be a non-empty string or null";
    } else {
      assignee = body.assignee.trim();
    }
  }

  if (Object.keys(fields).length > 0) {
    return err(validationError("invalid task payload", fields));
  }
  return ok({ title, description, priority, tags, assignee });
}

/** Validate a partial patch payload. All fields optional. */
export function validateTaskPatch(raw: unknown): Result<Partial<NewTaskInput>, DomainError> {
  if (typeof raw !== "object" || raw === null) {
    return err(validationError("body must be an object"));
  }
  const body = raw as Record<string, unknown>;
  const out: Partial<NewTaskInput> = {};
  const fields: Record<string, string> = {};

  if (body.title !== undefined) {
    if (typeof body.title !== "string" || isBlank(body.title)) {
      fields.title = "title must be a non-empty string";
    } else {
      out.title = body.title.trim();
    }
  }
  if (body.priority !== undefined) {
    if (!isPriority(body.priority)) {
      fields.priority = "invalid priority";
    } else {
      out.priority = body.priority;
    }
  }
  if (body.description !== undefined && typeof body.description === "string") {
    out.description = body.description;
  }

  if (Object.keys(fields).length > 0) {
    return err(validationError("invalid patch payload", fields));
  }
  return ok(out);
}
