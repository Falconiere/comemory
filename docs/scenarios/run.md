# run

**HTTP:** `POST /api/v1/run/embedding`

`comemory run embedding` explicitly reconciles supported code and documents in
exactly the invocation's current directory. Git and initialization are optional.
It never follows an ancestor configuration to expand the selected scope.

### Optional folder and metadata

**Flags:** `--path`, `--repo`, `--mode`

`--mode incremental` retries missing embeddings and changed content; `--mode full`
refreshes selected derived data. No flags and `--path .` select the same files. `--path` accepts relative or
absolute directory paths, including spaces; `--repo` supplies metadata and
never selects a different folder. Global flags are documented in globals.md.

**Covered by:** `tests/cli__run_embedding.rs::optional_flags_select_exact_folder_without_git`

### Reconciliation and source isolation

Renamed and deleted files lose their old derived rows on an authoritative walk;
other registered roots remain intact.

**Covered by:** `tests/cli__run_embedding.rs::renamed_and_deleted_files_reconcile_only_the_selected_root`
