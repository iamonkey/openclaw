/**
 * load.ts
 *
 * Configuration loader. Merges DEFAULT_CONFIG with values read from an
 * environment bag, validates the result, and returns a Result so the
 * bootstrap can fail fast on bad config.
 */

import { err, ok, type Result } from "../util/result";
import type { LogLevel } from "../util/logger";
import {
  readBool,
  readEnum,
  readInt,
  readString,
  type EnvBag,
} from "./env";
import {
  type AppConfig,
  DEFAULT_CONFIG,
  VALID_ENVS,
  VALID_LOG_LEVELS,
} from "./schema";

/** Build an AppConfig from an environment bag, applying defaults. */
export function loadConfig(env: EnvBag): Result<AppConfig> {
  const d = DEFAULT_CONFIG;

  const config: AppConfig = {
    env: readEnum(env, "APP_ENV", VALID_ENVS, d.env),
    logLevel: readEnum<LogLevel>(env, "LOG_LEVEL", VALID_LOG_LEVELS, d.logLevel),
    server: {
      host: readString(env, "HOST", d.server.host),
      port: readInt(env, "PORT", d.server.port),
      defaultPageSize: readInt(env, "DEFAULT_PAGE_SIZE", d.server.defaultPageSize),
      maxPageSize: readInt(env, "MAX_PAGE_SIZE", d.server.maxPageSize),
    },
    features: {
      enableSearch: readBool(env, "FEATURE_SEARCH", d.features.enableSearch),
      enableAssignees: readBool(env, "FEATURE_ASSIGNEES", d.features.enableAssignees),
    },
  };

  return validateConfig(config);
}

/** Validate cross-field invariants on a fully-built config. */
export function validateConfig(config: AppConfig): Result<AppConfig> {
  if (config.server.port < 1 || config.server.port > 65535) {
    return err(`invalid port: ${config.server.port}`);
  }
  if (config.server.defaultPageSize < 1) {
    return err("defaultPageSize must be >= 1");
  }
  if (config.server.maxPageSize < config.server.defaultPageSize) {
    return err("maxPageSize must be >= defaultPageSize");
  }
  return ok(config);
}
