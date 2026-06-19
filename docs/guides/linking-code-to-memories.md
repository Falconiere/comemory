# Linking code to memories

**Goal:** attach an explicit, version-pinned link from a memory to a specific
file or symbol, and read its freshness back on `comemory context`.

A memory often *is about* a function or file. Burying a `` `repo:path:sym` ``
token in the prose body works, but the link is version-blind: when the code
moves or changes you get no signal. The `--ref-file` / `--ref-symbol` flags make
the link explicit and **version-pinned**, so fetch can tell you whether the pin
still holds.

## Save with a reference

Run `comemory save` from inside the git repo the code lives in:

```bash
# Pin a file
comemory save "auth flow lives here" --ref-file src/auth/login.rs

# Pin a symbol (path:symbol)
comemory save "rate-limit lives in throttle()" \
  --ref-symbol src/http/limit.rs:throttle
```

Both flags are repeatable and comma-splittable. A value with no leading `repo:`
segment uses the save's resolved repo; paths are normalized to be repo-root
relative, so saving from a subdirectory produces the same reference as saving
from the repo root.

When the referenced path is **tracked** in the current repo, comemory captures a
**versioned anchor** at save time — the file's HEAD-tree blob OID, the HEAD
commit, and the branch — and stores it in the markdown frontmatter (the source
of truth). It records the last *committed* state even if your working copy is
dirty. No file bytes are snapshotted; the ref points at live code.

A missing-on-disk, untracked, or cross-repo reference is **advisory, never
fatal**: the save still exits 0, printing a `warning:` to stderr (or adding a
`warnings[]` entry in `--json` mode). Such a ref simply has no anchor and reads
back as `unpinned`.

## Read freshness on context

`comemory context <query>` returns each referenced code location with a `status`:

| status     | meaning                                                            |
|------------|-------------------------------------------------------------------|
| `fresh`    | the pinned blob matches the file's current HEAD blob              |
| `stale`    | the committed file changed since the anchor was taken            |
| `ghost`    | the target is gone (file removed, or symbol absent from a current index) |
| `unpinned` | no anchor was captured at save                                    |
| `unknown`  | pinned but unverifiable now (repo not on disk, or index absent/stale) |

File refs are classified independently of the code index (it's a pure git blob
compare). A symbol **ghost** is only reported when a *current* index covers the
file; otherwise the verdict degrades to `unknown` rather than a false `ghost`.

## Ghosts are prune-eligible

A memory whose pinned `--ref-symbol` resolves to a `ghost` — the symbol no
longer exists in a current index — is surfaced by `comemory prune` under
`ghost_ref_memories`. This is **advisory**: prune lists these memories so you
can update or drop the stale link; it never deletes them automatically. Re-save
the memory with the bad ref removed (or pointed at the symbol's new home) to
clear it.

## See also

- [Architecture §7.2](../architecture.md) — the anchor + staleness model.
- [Bring your own vectors](byo-vectors.md) — adding semantic retrieval.
