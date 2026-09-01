#!/usr/bin/env bash
# Regenerate tests/golden/ranking-invariance.json.
#
# WHY THIS EXISTS: the console-compat change adds pre-fusion leg scores to
# `score_parts` (spec AC-19..AC-23). Those values are *carried through* from
# leg vectors the router already holds, so no ranking may move — but a snapshot
# written by the same commit that edits the ranker cannot testify about the
# ranking before it. So this fixture is generated from the PRE-change binary and
# committed first; the replay test (tests/ranking_invariance.rs) then re-runs the
# same corpus and queries afterwards and demands the same ordering.
#
# The fixture carries the corpus AND the queries AND the expected output, so the
# replay test has exactly one source of truth to rebuild from — the corpus is
# never restated in Rust.
#
# Usage:  bash tests/golden/ranking-invariance.gen.sh [path/to/comemory]
# Default binary: target/debug/comemory (built from the CURRENT checkout — run
# this BEFORE editing src/retrieval/).

set -euo pipefail

BIN="${1:-target/debug/comemory}"
OUT="tests/golden/ranking-invariance.json"

if [[ ! -x "$BIN" ]]; then
    echo "ranking-invariance.gen: no binary at $BIN — run 'cargo build' first" >&2
    exit 1
fi
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

# Access tracking mutates access_count/last_accessed, which feeds ACT-R
# activation and reorders later queries. Without this the generator is not
# reproducible and neither is the replay.
export COMEMORY_DISABLE_ACCESS_TRACKING=true

DATA_DIR="$(mktemp -d)"
trap 'rm -rf "$DATA_DIR"' EXIT
export COMEMORY_DATA_DIR="$DATA_DIR"

# One corpus entry per line: <kind>|<tags>|<quality>|<body>
CORPUS=(
"decision|frontmatter,schema|5|YAML frontmatter is the contract, not the body. Everything the ranker needs lives in frontmatter; the body is free prose indexed only for FTS."
"bug|frontmatter,serde|3|Frontmatter round-trip broke on empty tag lists. An empty sequence serialized as null and failed to parse back."
"decision|retrieval,fusion|5|RRF k=60 beat every weighted-sum fusion we tried. Rank fusion is scale-free, so a noisy leg cannot dominate."
"convention|sqlite,embed|4|Never call the embedder inside a transaction. The write lock is held for the whole HTTP round trip and every other writer stalls."
"discovery|fts5,tokenizer|4|The FTS5 tokenizer must split on digit boundaries too, or an identifier like parse2json is unreachable from parse."
"discovery|graph,cochange|3|Co-change edges decay faster than import edges, because a refactor rewrites imports but leaves history behind."
"convention|guardrails,house-rules|5|One responsibility per file, at most 300 lines, no exceptions. A module that outgrows the ceiling splits before it lands."
"decision|prune,trash|4|Soft-delete moves the markdown first and never touches the database first. Markdown is the source of truth, so it wins the race."
"note|sqlite,wal|2|WAL mode means page_count times page_size diverges from the file length on disk until a checkpoint runs."
"pattern|retrieval,ladder|4|The lexical ladder only adds recall on an empty result. A non-empty earlier tier short-circuits, so it can never reorder stricter hits."
"bug|migrate,backup|3|A failed snapshot before a destructive migration must refuse the upgrade, not warn. Warning once reported a clean bill of health on a broken database."
"decision|schema,compat|4|An older binary must refuse a newer schema rather than guess at unknown markers, because a silent partial read corrupts the mirror."
)

# Queries are chosen for RANKED DEPTH, not for reading well: a one-hit result
# cannot detect a reordering, so a query that returns a single memory proves
# nothing. Multi-term queries also reach the word-OR fallback (tier 2), which
# puts the lexical ladder itself under the invariance check rather than only
# the strict tier.
QUERIES=(
"migration schema database"
"file lines index"
"schema"
"frontmatter"
"database"
"fusion rank retrieval leg"
"markdown source truth database"
"parse2json"
)

for entry in "${CORPUS[@]}"; do
    IFS='|' read -r kind tags quality body <<<"$entry"
    "$BIN" save "$body" --kind "$kind" --tags "$tags" --quality "$quality" \
        --repo comemory >/dev/null
done

python3 - "$BIN" "$OUT" "${QUERIES[@]}" <<'PY'
import json, os, subprocess, sys

binary, out = sys.argv[1], sys.argv[2]
queries = sys.argv[3:]

corpus = []
for name in sorted(os.listdir(os.path.join(os.environ["COMEMORY_DATA_DIR"], "memories"))):
    if name.endswith(".md"):
        with open(os.path.join(os.environ["COMEMORY_DATA_DIR"], "memories", name)) as fh:
            corpus.append({"file": name, "markdown": fh.read()})

expected = []
for q in queries:
    raw = subprocess.run(
        [binary, "search", q, "--json", "--k", "8"],
        capture_output=True, text=True,
    )
    if raw.returncode != 0:
        sys.exit(f"search {q!r} failed ({raw.returncode}): {raw.stderr}")
    raw = raw.stdout
    payload = json.loads(raw)
    expected.append({
        "query": q,
        "hits": [
            {
                "memory_id": h["memory_id"],
                "score": h["score"],
                "tier": h["tier"],
                "score_parts": h["score_parts"],
            }
            for h in payload["hits"]
        ],
    })

with open(out, "w") as fh:
    json.dump(
        {
            "note": "Generated by tests/golden/ranking-invariance.gen.sh from the "
                    "pre-change binary. Replayed by tests/ranking_invariance.rs. "
                    "Ordering must never move; see spec AC-22.",
            "corpus": corpus,
            "expected": expected,
        },
        fh, indent=2,
    )
    fh.write("\n")
print(f"wrote {out}: {len(corpus)} memories, {len(expected)} queries")
PY
