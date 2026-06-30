#!/usr/bin/env node
/**
 * tokens.mjs
 *
 * Dependency-free token estimator that mirrors the Rust `estimate_tokens`
 * heuristic used by carto-core: ceil(bytes / 4), with the empty string
 * counting as 0 tokens. Keeping the JS estimate byte-identical to the Rust
 * one lets the benchmark harness compare baseline (grep + full-file) token
 * cost against the Cartograph arm apples-to-apples.
 *
 * Library use:
 *   import { estimateTokens } from "./tokens.mjs";
 *   estimateTokens("hello world");  // -> 3
 *
 * CLI use:
 *   node tokens.mjs <file...>       // prints per-file + total estimated tokens
 *
 * Note: the Rust heuristic counts BYTES, not UTF-16 code units. For ASCII
 * source these are identical; for multibyte content we measure UTF-8 byte
 * length so the estimate matches Rust's `text.len()`.
 */

import { readFileSync } from "node:fs";

/**
 * Estimate token count for a string as ceil(byteLength / 4).
 * @param {string} text
 * @returns {number}
 */
export function estimateTokens(text) {
  return text.length === 0 ? 0 : Math.ceil(byteLength(text) / 4);
}

/** UTF-8 byte length of a string (matches Rust `str::len`). */
function byteLength(text) {
  return Buffer.byteLength(text, "utf8");
}

function runCli(args) {
  if (args.length === 0) {
    process.stderr.write("usage: node tokens.mjs <file...>\n");
    process.exitCode = 1;
    return;
  }

  let total = 0;
  let failed = false;
  for (const file of args) {
    try {
      const text = readFileSync(file, "utf8");
      const tokens = estimateTokens(text);
      total += tokens;
      process.stdout.write(`${tokens}\t${file}\n`);
    } catch (err) {
      failed = true;
      process.stderr.write(`error: cannot read ${file}: ${err.message}\n`);
    }
  }
  process.stdout.write(`${total}\ttotal\n`);
  if (failed) process.exitCode = 1;
}

// Run as CLI when invoked directly (not when imported).
const invokedPath = process.argv[1] ? new URL(`file://${process.argv[1]}`).pathname : "";
if (invokedPath.endsWith("tokens.mjs")) {
  runCli(process.argv.slice(2));
}
