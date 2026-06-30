/**
 * logger.ts
 *
 * Structured logger used by the HTTP layer and the server bootstrap.
 * Levels are filtered against a configured threshold. The logger is
 * intentionally synchronous and writes JSON lines so they are greppable.
 */

export type LogLevel = "debug" | "info" | "warn" | "error";

const ORDER: Record<LogLevel, number> = {
  debug: 10,
  info: 20,
  warn: 30,
  error: 40,
};

export interface LogFields {
  [key: string]: unknown;
}

export class Logger {
  constructor(
    private readonly threshold: LogLevel = "info",
    private readonly sink: (line: string) => void = (l) => console.log(l),
  ) {}

  private enabled(level: LogLevel): boolean {
    return ORDER[level] >= ORDER[this.threshold];
  }

  private emit(level: LogLevel, msg: string, fields?: LogFields): void {
    if (!this.enabled(level)) return;
    const record = { level, msg, ts: new Date().toISOString(), ...fields };
    this.sink(JSON.stringify(record));
  }

  debug(msg: string, fields?: LogFields): void {
    this.emit("debug", msg, fields);
  }

  info(msg: string, fields?: LogFields): void {
    this.emit("info", msg, fields);
  }

  warn(msg: string, fields?: LogFields): void {
    this.emit("warn", msg, fields);
  }

  error(msg: string, fields?: LogFields): void {
    this.emit("error", msg, fields);
  }

  child(threshold: LogLevel): Logger {
    return new Logger(threshold, this.sink);
  }
}

export const defaultLogger = new Logger();
