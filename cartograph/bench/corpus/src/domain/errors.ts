/**
 * errors.ts
 *
 * Domain error taxonomy. These are plain tagged objects (not thrown
 * exceptions) carried inside Result.error so the HTTP layer can map them
 * to status codes deterministically.
 */

export type DomainErrorCode =
  | "not_found"
  | "validation"
  | "conflict"
  | "forbidden_transition";

export interface DomainError {
  code: DomainErrorCode;
  message: string;
  /** Optional per-field validation details. */
  fields?: Record<string, string>;
}

export function notFound(message: string): DomainError {
  return { code: "not_found", message };
}

export function validationError(message: string, fields?: Record<string, string>): DomainError {
  return { code: "validation", message, fields };
}

export function conflict(message: string): DomainError {
  return { code: "conflict", message };
}

export function forbiddenTransition(message: string): DomainError {
  return { code: "forbidden_transition", message };
}

/** Map a domain error code to an HTTP status. Consumed by the router. */
export function statusForError(code: DomainErrorCode): number {
  switch (code) {
    case "not_found":
      return 404;
    case "validation":
      return 422;
    case "conflict":
      return 409;
    case "forbidden_transition":
      return 409;
    default:
      return 500;
  }
}
