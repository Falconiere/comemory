# `comemory save`

Write a memory as markdown (the source of truth) and upsert the SQLite
mirror: FTS, optional vector, relation edges, and code refs.

**Runnable tests:** `tests/cli__save.rs`, `tests/cli__save_2.rs`,
`tests/cli__ref_args.rs`, `tests/cli_scenario_memory_lifecycle.rs`

**HTTP:** `POST /api/v1/memories` — covered by `tests/serve_scenario_memory_lifecycle.rs`, `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_vectors.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`[BODY]` — memory body. Omit or pass `-` to read stdin.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--kind` | `note` | `decision` \| `bug` \| `convention` \| `discovery` \| `pattern` \| `note` |
| `--repo` | empty | Free-form repo label stored on the row |
| `--tags` | empty | Comma-separated tag list |
| `--author` | empty | Author identifier |
| `--quality` | `3` | Integer 1..=5. Out of range is clap usage |
| `--supersedes` | empty | Comma-separated 8-hex ids this memory replaces |
| `--vector` | unset | CSV of f32; length must equal the memory dim (1024) |
| `--vector-stdin` | off | JSON `{"embedding":[..]}` on stdin. Body must be a positional arg |
| `--ref-file` | unset | Repeatable `[repo:]path` version-anchored file ref |
| `--ref-symbol` | unset | Repeatable `[repo:]path:symbol`. Missing `:symbol` is usage |

## Scenarios

### save-01 Positional body

- **Flags:** `--kind` `--repo` `--tags` `--author`
- **Setup:** empty data dir
- **Command:**

```bash
comemory save "advisory locks for migration ordering" \
  --kind note --repo foo --tags db,postgres --author alice
```

- **Expect:** TTY `saved <8-hex>`; one `.md` under `memories/`; FTS row; no
  `memory_vec` row (lexical-only).
- **Covered by:** `tests/cli__save.rs::save_writes_md_and_indexes_lexical_when_no_vector`

### save-02 Stdin body

- **Flags:** `-` (stdin body)
- **Setup:** empty data dir
- **Command:**

```bash
echo "piped stdin body" | comemory save - --kind note --json
```

- **Expect:** `id` is 8 hex. `show` round-trips the body.
- **Covered by:** `tests/cli_scenario_memory_lifecycle.rs`

### save-03 All six kinds

- **Flags:** `--kind` (each value)
- **Setup:** empty data dir
- **Command:** `comemory save "<kind> body" --kind <kind> --repo alpha` for
  each of the six kinds.
- **Expect:** `list --json` contains every kind.
- **Covered by:** `tests/cli_scenario_memory_lifecycle.rs`

### save-04 Quality bounds

- **Flags:** `--quality`
- **Command:** `comemory save body --quality 99`
- **Expect:** non-zero exit; stderr mentions `5`.
- **Covered by:** `tests/cli__save_2.rs::save_rejects_out_of_range_quality`

### save-05 Unknown kind

- **Flags:** `--kind`
- **Command:** `comemory save body --kind banana`
- **Expect:** clap usage failure (not silently coerced to `note`).
- **Covered by:** `tests/cli__save_2.rs::save_rejects_unknown_kind`

### save-06 Vector stdin

- **Flags:** `--vector-stdin`
- **Setup:** 1024-dim JSON embedding
- **Command:**

```bash
echo '{"embedding":[...1024 floats...]}' | comemory save "body" --vector-stdin
```

- **Expect:** one `memory_vec` row. Wrong dim aborts before the markdown write.
- **Covered by:** `tests/cli__save.rs::save_with_vector_stdin_writes_memory_vec_row`,
  `tests/cli__save_2.rs::save_rejects_wrong_dim_vector`

### save-07 Vector CSV

- **Flags:** `--vector`
- **Command:** `comemory save body --vector 0.1,0.2,...`
- **Expect:** one `memory_vec` row. A non-float token fails parse.
- **Covered by:** `tests/cli__save.rs::save_with_vector_csv_flag_writes_memory_vec_row`

### save-08 Supersedes

- **Flags:** `--supersedes`
- **Setup:** an existing memory id
- **Command:** `comemory save "v2 body" --supersedes <old-id>`
- **Expect:** edge `new → old`; search annotates `superseded_by`;
  self-supersede and malformed ids are rejected before write.
- **Covered by:** `tests/cli__save_2.rs::save_supersedes_writes_edge_frontmatter_and_penalizes_ranking`

### save-09 Near-duplicate advisory

- **Flags:** _(none extra)_
- **Setup:** a SimHash-near body already saved
- **Command:** `comemory save "<near-dup body>"`
- **Expect:** TTY stderr warning; `--json` includes `duplicate_of`. The save
  still proceeds.
- **Covered by:** `tests/cli__save_2.rs::near_duplicate_save_tty_emits_warning_line`

### save-10 Ref symbol

- **Flags:** `--ref-symbol`
- **Setup:** cwd is a git repo; the path is committed
- **Command:** `comemory save body --ref-symbol src/lib.rs:foo`
- **Expect:** frontmatter + `code_ref` row + edge. A value without `:symbol`
  is usage (exit 64).
- **Covered by:** `tests/cli__ref_args.rs`

### save-11 Ref file untracked

- **Flags:** `--ref-file`
- **Setup:** path not in the git tree
- **Command:** `comemory save body --ref-file untracked.txt`
- **Expect:** exit 0, unpinned, advisory warning.
- **Covered by:** `tests/cli__ref_args.rs::untracked_path_is_unpinned_with_warning`
