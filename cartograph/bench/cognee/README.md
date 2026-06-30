# cognee head-to-head harness

A reproducible harness to benchmark [cognee](https://github.com/topoteretes/cognee)
against Cartograph on the **same corpus + tasks** (`../corpus`, `../tasks.json`),
plus what we found trying to run it in the Cartograph dev sandbox.

## What we did (and what happened)

| step | result |
|---|---|
| `pip install cognee` | **timed out** first try (heavy dep stack via the egress proxy). |
| retry, longer timeout | got further, then **failed building `langdetect`** (`AttributeError: install_layout` — Debian-patched setuptools). |
| venv + `setuptools<66` + `SETUPTOOLS_USE_DISTUTILS=stdlib` | **installed cognee 1.2.2** cleanly (pulled `openai`, `litellm`, `lancedb`, `tiktoken`, `instructor`, …). |
| run minimal `cognee.add(snippet)` + `cognify()` + `search()` | **blocked** at the very first `add()` with `LLMAPIKeyNotSetError`. |

The stack trace is the finding:

```
cognee.add(...)
  -> modules/pipelines/.../setup_and_check_environment.py: setup_and_check_environment(...)
  -> infrastructure/llm/utils.py: test_llm_connection()
  -> LLMAPIKeyNotSetError: LLM API key is not set. (Status code: 422)
```

**cognee requires an LLM API key in the critical path to even ingest a
document** — `add()` checks the LLM connection during pipeline setup, before any
cognify/embed. There is no offline path. This empirically confirms the
architectural point in `../../docs/comparison-vs-prior-art.md`: cognee is
LLM-in-the-loop at *ingest* (and therefore at *re-index on edit*), whereas
Cartograph indexed 257k LOC and resyncs a single-file edit in ~15 ms p50 with
**zero model calls** (`../REPORT-LONGHORIZON.md`).

Because this sandbox has no LLM key (and the same egress class that blocks
`cognee.ai`/`arxiv.org` would block model/provider traffic), a token/accuracy
head-to-head cannot run here. The harness below produces one wherever a key +
egress exist.

## Running the real head-to-head

```sh
python3 -m venv venv && . venv/bin/activate
SETUPTOOLS_USE_DISTUTILS=stdlib pip install "setuptools<66" wheel
SETUPTOOLS_USE_DISTUTILS=stdlib pip install cognee tiktoken

export LLM_API_KEY=sk-...            # cognee defaults to OpenAI; see cognee docs
# optional: export LLM_PROVIDER=... LLM_MODEL=... EMBEDDING_MODEL=...

python run_cognee_bench.py ../corpus ../tasks.json results.cognee.json
```

It ingests the corpus (`add` + `cognify`), then for each task runs `cognee.search`
and records latency, returned-context token size (tiktoken `cl100k_base`), and
whether the ground-truth file/symbol appears in the result.

## Comparing the numbers fairly

- **Token metric differs.** Cartograph's `carto-bench` counts the tokens an agent
  must *read* to resolve a task (`ceil(bytes/4)`, identical on baseline and
  Cartograph). This harness counts tiktoken tokens of cognee's returned context.
  Use ratios and orders of magnitude, not raw equality.
- **Ingest vs query.** The number that matters for a long-horizon *editing* agent
  is re-index-on-edit cost. cognee's `add`+`cognify` is the comparable figure to
  Cartograph's incremental resync (`carto sync`, ~15 ms p50 on 257k LOC). Expect
  cognee's per-file ingest to be orders of magnitude higher (LLM + embed).
- **Different goals.** cognee also offers cross-session/persistent memory and
  fuzzy NL recall that Cartograph deliberately does not — see the comparison doc.
  This harness measures code-retrieval, the axis they overlap on.
