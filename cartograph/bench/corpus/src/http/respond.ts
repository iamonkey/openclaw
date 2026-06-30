/**
 * respond.ts
 *
 * Helpers to turn domain Results and errors into HttpResponses. Centralizes
 * the error -> status mapping so every handler responds consistently.
 */

import { type DomainError, statusForError } from "../domain/errors";
import { json, type HttpResponse } from "./types";

/** Wrap a successful payload. */
export function okResponse(body: unknown, status = 200): HttpResponse {
  return json(status, body);
}

/** Map a DomainError to its HTTP response with a stable error envelope. */
export function errorResponse(error: DomainError): HttpResponse {
  return json(statusForError(error.code), {
    error: {
      code: error.code,
      message: error.message,
      fields: error.fields ?? undefined,
    },
  });
}

/** Generic 400 for malformed requests the router itself rejects. */
export function badRequest(message: string): HttpResponse {
  return json(400, { error: { code: "bad_request", message } });
}
