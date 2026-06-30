/**
 * load.test.ts
 *
 * Tests for configuration loading and validation. Uses a plain env bag so
 * process.env is never touched.
 */

import { describe, expect, it } from "vitest";
import { isErr, isOk } from "../util/result";
import { loadConfig } from "./load";

describe("loadConfig", () => {
  it("falls back to defaults on an empty env", () => {
    const r = loadConfig({});
    expect(isOk(r)).toBe(true);
    if (isOk(r)) {
      expect(r.value.server.port).toBe(8080);
      expect(r.value.logLevel).toBe("info");
    }
  });

  it("reads overrides from the env bag", () => {
    const r = loadConfig({ PORT: "9090", LOG_LEVEL: "debug", FEATURE_SEARCH: "0" });
    expect(isOk(r)).toBe(true);
    if (isOk(r)) {
      expect(r.value.server.port).toBe(9090);
      expect(r.value.logLevel).toBe("debug");
      expect(r.value.features.enableSearch).toBe(false);
    }
  });

  it("rejects an out-of-range port", () => {
    const r = loadConfig({ PORT: "70000" });
    expect(isErr(r)).toBe(true);
  });

  it("rejects maxPageSize below defaultPageSize", () => {
    const r = loadConfig({ DEFAULT_PAGE_SIZE: "50", MAX_PAGE_SIZE: "10" });
    expect(isErr(r)).toBe(true);
  });
});
