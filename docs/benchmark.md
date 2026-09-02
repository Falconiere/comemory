# Benchmarking token efficiency

Status (2026-09-02): planned, not yet run.

The claim worth a number: an agent with comemory reads fewer tokens and
makes fewer tool calls to finish a task than the same agent grepping and
catting files. This page records what exists to measure it and the first
experiment that would produce the number.

## What exists off the shelf

Nothing measures the token savings of one specific code-retrieval tool out
of the box. The closest substitutes, and what each gives us:

| Resource | What it gives us |
| --- | --- |
| [letya999/tools-token-economy](https://github.com/letya999/tools-token-economy) | A runnable harness: 21 tool configurations (grep, ripgrep, LSP, RAG, semantic) benchmarked on the same task and model, reporting tokens, tool calls, and a waste ratio. Config-driven, so a `comemory` config is the extension point. |
| [Sverklo: "I benchmarked code retrieval for AI agents"](https://sverklo.com/blog/i-benchmarked-code-retrieval-for-ai-agents/) | 60 tasks, grep vs MCP retrieval; publishes a tokens / tool-calls / F1 table and a "tokens per correct answer" metric. The reporting template to copy. |
| [LatentEval: "Is grep all you need?"](https://latenteval.ai/research/is-grep-all-you-need) | Methodology caution: the harness and the delivery mode (inline vs file) swing results as much as the retriever does. Pin both. |
| [SWE-bench / SWE-bench Lite](https://github.com/SWE-bench/SWE-bench/) | Issue text → gold touched files on real repos. The best match for comemory's repo-local use. |
| [CoIR](https://github.com/CoIR-team/coir) | CodeSearchNet, CosQA and friends in one BEIR-shaped schema; recall@k via `pytrec_eval`. Generic NL → function retrieval. |
| [Agent Retrieval Bench](https://export.arxiv.org/pdf/2607.24882) | File-level workflow queries with gold rankings and a token-budget metric (BCY@B) worth reusing verbatim. |
| [Claude Code headless](https://code.claude.com/docs/en/headless) | `claude -p … --output-format json` returns `usage.input_tokens`, `usage.output_tokens`, `num_turns`; `--allowedTools` is the on/off knob for the A/B. |
| [nilenso/swe-bench-pro-cost-token-time-analysis](https://github.com/nilenso/swe-bench-pro-cost-token-time-analysis) | Paired-task token / cost / time statistics from real trajectories; a sanity range for our numbers. |

## Minimal first experiment (one day)

1. Pick 20–30 SWE-bench Lite instances across 2–3 repos. `comemory
   index-code` each repo at the instance's base commit.
2. For every instance run `claude -p "<issue text>" --output-format json`
   twice, same model, same task, same repo state:
   - **A (baseline):** `--allowedTools "Bash,Read,Grep,Glob"`.
   - **B (comemory):** the same set plus comemory's CLI (`search-code`,
     `context`, `find`) reachable from `Bash`, with the agent told it exists.
3. Capture per run, straight from the JSON result: `usage.input_tokens`,
   `usage.output_tokens`, `num_turns` (or count `tool_use` blocks in the
   JSONL transcript under `~/.claude/projects/` for exact tool calls).
4. Capture task success against the SWE-bench gold patch / tests, and
   recall@k of the gold files among the files the agent opened.
5. Report the same triad both published baselines use, so the numbers are
   comparable: mean input tokens per task, mean tool calls per task, task
   success rate; plus tokens per correct answer.

## Why it has not run yet

The runs are headless API calls against dozens of tasks, twice each: real
model spend and wall-clock that belong to a deliberate session. Everything
needed to run it is listed above; `scripts/` gets a driver when the
experiment is scheduled, and this page gets the resulting table.
