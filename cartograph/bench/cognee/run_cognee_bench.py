#!/usr/bin/env python3
"""Head-to-head harness: cognee vs Cartograph on the same corpus + tasks.

Mirrors cartograph/crates/carto-bench so the two are comparable:
  - Ingest the corpus into cognee (add + cognify) and measure ingest time/tokens.
  - For each task in tasks.json, run cognee.search and measure:
      * latency, and
      * the token size of the returned context (tiktoken), and
      * localization: did the ground-truth file/symbol appear in the result.
  - Emit results.cognee.json.

REQUIREMENTS (cognee is LLM-in-the-loop; none of this runs without them):
  - `pip install cognee` (use SETUPTOOLS_USE_DISTUTILS=stdlib if langdetect fails).
  - An LLM API key: export LLM_API_KEY=... (and LLM_PROVIDER/LLM_MODEL as needed;
    cognee defaults to OpenAI). cognee checks the LLM connection at ingest time.
  - Network egress to the LLM provider + first-run model/tokenizer downloads.

In the Cartograph dev sandbox this harness reaches `cognee.add()` and stops at
`LLMAPIKeyNotSetError` — see README.md. Run it in an environment that has a key
and egress to get real numbers.

Usage:
  python run_cognee_bench.py <corpus_dir> <tasks.json> [out.json]
"""

import asyncio
import json
import sys
import time
from pathlib import Path

try:
    import tiktoken
    _ENC = tiktoken.get_encoding("cl100k_base")
    def ntok(s: str) -> int:
        return len(_ENC.encode(s))
except Exception:
    # Fall back to the same ceil(bytes/4) estimate Cartograph uses, so numbers
    # stay comparable if tiktoken's encoder can't be downloaded.
    def ntok(s: str) -> int:
        b = len(s.encode("utf-8"))
        return 0 if b == 0 else (b + 3) // 4


def load_corpus(corpus: Path):
    files = []
    for p in sorted(corpus.rglob("*.ts")):
        try:
            files.append((str(p.relative_to(corpus)).replace("\\", "/"), p.read_text()))
        except Exception:
            pass
    return files


def result_to_text(res) -> str:
    """cognee.search returns a list of heterogeneous results; flatten to text."""
    if res is None:
        return ""
    if isinstance(res, str):
        return res
    try:
        return "\n".join(
            x if isinstance(x, str) else json.dumps(x, default=str) for x in res
        )
    except Exception:
        return str(res)


async def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    corpus = Path(sys.argv[1]).resolve()
    tasks = json.loads(Path(sys.argv[2]).read_text())
    out_path = Path(sys.argv[3]) if len(sys.argv) > 3 else Path("results.cognee.json")

    import cognee

    files = load_corpus(corpus)
    corpus_tokens = sum(ntok(t) for _, t in files)
    print(f"corpus: {len(files)} files, ~{corpus_tokens} tokens", file=sys.stderr)

    # ── Ingest (the freshness-relevant cost: LLM extract + embed per file) ────
    t0 = time.time()
    for rel, text in files:
        # Tag each chunk with its path so localization is checkable.
        await cognee.add(f"// file: {rel}\n{text}")
    await cognee.cognify()
    ingest_s = time.time() - t0
    print(f"ingest (add+cognify): {ingest_s:.1f}s", file=sys.stderr)

    # ── Per-task search ───────────────────────────────────────────────────────
    results = []
    for task in tasks:
        q0 = time.time()
        try:
            res = await cognee.search(task["query"])
            err = None
        except Exception as e:  # surface, don't crash the whole run
            res, err = None, repr(e)
        latency_ms = (time.time() - q0) * 1000.0
        text = result_to_text(res)
        ctx_tokens = ntok(text)
        sym_hit = any(s in text for s in task.get("answer_symbols", []))
        file_hit = any(f in text for f in task.get("answer_files", []))
        results.append({
            "id": task["id"], "class": task.get("class"),
            "latency_ms": latency_ms, "context_tokens": ctx_tokens,
            "symbol_hit": sym_hit, "file_hit": file_hit, "error": err,
        })
        print(f"  {task['id']}: {ctx_tokens} tok, {latency_ms:.0f}ms, "
              f"sym={sym_hit} file={file_hit}", file=sys.stderr)

    n = max(len(results), 1)
    summary = {
        "corpus_files": len(files),
        "corpus_tokens": corpus_tokens,
        "ingest_seconds": ingest_s,
        "mean_context_tokens": sum(r["context_tokens"] for r in results) / n,
        "symbol_localization": sum(r["symbol_hit"] for r in results),
        "file_localization": sum(r["file_hit"] for r in results),
        "tasks": results,
    }
    out_path.write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
