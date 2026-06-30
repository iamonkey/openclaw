/**
 * schema.ts
 *
 * The shape of application configuration plus its defaults. Loaded once at
 * startup (see load.ts) and threaded into the server and HTTP layers.
 */

import type { LogLevel } from "../util/logger";

export interface ServerConfig {
  host: string;
  port: number;
  /** Default page size when a list request omits one. */
  defaultPageSize: number;
  maxPageSize: number;
}

export interface AppConfig {
  env: "development" | "test" | "production";
  logLevel: LogLevel;
  server: ServerConfig;
  /** Feature flags toggled by environment. */
  features: {
    enableSearch: boolean;
    enableAssignees: boolean;
  };
}

export const DEFAULT_CONFIG: AppConfig = {
  env: "development",
  logLevel: "info",
  server: {
    host: "127.0.0.1",
    port: 8080,
    defaultPageSize: 20,
    maxPageSize: 100,
  },
  features: {
    enableSearch: true,
    enableAssignees: true,
  },
};

export const VALID_LOG_LEVELS: LogLevel[] = ["debug", "info", "warn", "error"];
export const VALID_ENVS: AppConfig["env"][] = ["development", "test", "production"];
