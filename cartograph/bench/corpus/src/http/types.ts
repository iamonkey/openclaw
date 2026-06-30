/**
 * types.ts
 *
 * HTTP-ish request/response shapes. This codebase does not bind a real
 * server socket; instead handlers operate on these plain objects so they
 * are trivially testable. The router dispatches HttpRequest -> HttpResponse.
 */

export type HttpMethod = "GET" | "POST" | "PATCH" | "DELETE";

export interface HttpRequest {
  method: HttpMethod;
  path: string;
  /** Parsed path params (e.g. { id }) filled in by the router. */
  params: Record<string, string>;
  /** Parsed query string. */
  query: Record<string, string>;
  /** Parsed JSON body, if any. */
  body?: unknown;
}

export interface HttpResponse {
  status: number;
  body: unknown;
}

export type Handler = (req: HttpRequest) => HttpResponse;

export function json(status: number, body: unknown): HttpResponse {
  return { status, body };
}
