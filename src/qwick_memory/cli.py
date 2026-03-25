"""Typer CLI for qwick-memory: save, search, list, delete, index, doctor."""

from __future__ import annotations

import logging
import os
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import typer
from rich.console import Console
from rich.table import Table

from qwick_memory.config import (
  get_author,
  get_index,
  get_memories_dir,
  get_rag_dir,
  get_repo,
  get_vectordb_dir,
)
from qwick_memory.git_utils import git_sync
from qwick_memory.memory import (
  MEMORY_TYPES,
  Memory,
  generate_id,
  parse_memory,
  scan_memories,
  write_memory,
)
from qwick_memory.search import search_memories

app = typer.Typer(help="qwick-memory: Centralized RAG memory for multiple repositories.")
console = Console(stderr=True)
out = Console()


def _verbose_callback(value: bool) -> None:
  if value:
    logging.basicConfig(level=logging.DEBUG, format="%(name)s: %(message)s")


verbose_option = typer.Option(
  False,
  "--verbose",
  "-v",
  help="Enable verbose logging.",
  callback=_verbose_callback,
  is_eager=True,
)

TOKEN_WARN_LIMIT = 6000  # calibrated for nomic 8192-token context


def _open_editor() -> str | None:
  """Open $EDITOR for the user to type content. Returns text or None."""
  editor = os.environ.get("EDITOR", "vi")
  with tempfile.NamedTemporaryFile(suffix=".md", delete=False, mode="w") as f:
    tmp_path = f.name
  try:
    result = subprocess.run([editor, tmp_path], check=False)
    if result.returncode != 0:
      return None
    text = Path(tmp_path).read_text().strip()
    return text if text else None
  finally:
    Path(tmp_path).unlink(missing_ok=True)


@app.command()
def save(
  content: str | None = typer.Argument(None, help="Memory content (opens $EDITOR if omitted)."),
  type: str = typer.Option("note", "--type", "-t", help="Memory type."),
  tags: str = typer.Option("", "--tags", help="Comma-separated tags."),
  repo: str = typer.Option(
    "", "--repo", "-r", help="Comma-separated repos (auto-detected if omitted)."
  ),
  verbose: bool = verbose_option,
) -> None:
  """Save a new memory."""
  # Validate type
  if type not in MEMORY_TYPES:
    console.print(f"[red]Invalid type '{type}'. Must be one of: {', '.join(MEMORY_TYPES)}[/red]")
    raise typer.Exit(1)

  # Get content from argument or editor
  if content is None:
    content = _open_editor()
    if not content:
      console.print("[yellow]No content provided. Aborting.[/yellow]")
      raise typer.Exit(1)

  # Warn on long content
  word_count = len(content.split())
  if word_count > TOKEN_WARN_LIMIT:
    console.print(
      f"[yellow]Warning: content is {word_count} words "
      f"(>{TOKEN_WARN_LIMIT}). Consider splitting into smaller memories.[/yellow]"
    )

  # Generate ID and prepare memory
  memory_id = generate_id(content)
  tag_list = [t.strip() for t in tags.split(",") if t.strip()]
  if repo:
    repo_list = [r.strip() for r in repo.split(",") if r.strip()]
  else:
    detected = get_repo()
    if detected is None:
      console.print(
        "[red]Could not auto-detect repo (no .git in project root). "
        "Use --repo or set QWICK_MEMORY_REPO.[/red]"
      )
      raise typer.Exit(1)
    repo_list = [detected]
  author = get_author()

  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"

  # Skip if file already exists (duplicate content)
  if final_path.exists():
    console.print(f"[yellow]Memory already exists: {memory_id}[/yellow]")
    raise typer.Exit(0)

  memory = Memory(
    id=memory_id,
    repo=repo_list,
    type=type,
    tags=tag_list,
    author=author,
    created=datetime.now(timezone.utc),
    content=content,
  )

  # Atomic write: temp file -> embed -> upsert -> rename
  tmp_path = memories_dir / f".{memory_id}.tmp"
  try:
    write_memory(memory, tmp_path, memories_dir=memories_dir)
    idx = get_index()
    idx.upsert(memory)
    tmp_path.rename(final_path)
  except Exception as exc:
    tmp_path.unlink(missing_ok=True)
    console.print(f"[red]Failed to save memory: {exc}[/red]")
    raise typer.Exit(1) from exc

  git_sync(get_rag_dir(), f"save: {memory_id} ({type})")
  out.print(f"Saved memory [bold]{memory_id}[/bold]")


@app.command()
def search(
  query: str = typer.Argument(..., help="Search query."),
  repo: str | None = typer.Option(None, "--repo", "-r", help="Filter by repo."),
  type: str | None = typer.Option(None, "--type", "-t", help="Filter by type."),
  tag: str | None = typer.Option(None, "--tag", help="Filter by tag."),
  limit: int = typer.Option(10, "--limit", "-n", help="Max results."),
  verbose: bool = verbose_option,
) -> None:
  """Search memories by semantic similarity."""
  idx = get_index()
  results = search_memories(idx, query, repo=repo, type_filter=type, tag=tag, limit=limit)

  if not results:
    out.print("No results found.")
    return

  table = Table(title="Search Results")
  table.add_column("Score", justify="right", style="cyan")
  table.add_column("Repo", style="green")
  table.add_column("Type", style="magenta")
  table.add_column("Content", style="white")
  table.add_column("ID", style="dim")

  for r in results:
    preview = r.content[:80] + "..." if len(r.content) > 80 else r.content
    table.add_row(
      f"{r.score:.3f}",
      r.repo,
      r.type,
      preview,
      r.id,
    )

  out.print(table)


@app.command(name="list")
def list_memories(
  repo: str | None = typer.Option(None, "--repo", "-r", help="Filter by repo."),
  type: str | None = typer.Option(None, "--type", "-t", help="Filter by type."),
  tags: str | None = typer.Option(None, "--tags", help="Filter by tags (comma-separated)."),
  verbose: bool = verbose_option,
) -> None:
  """List memories from disk (not the index)."""
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    out.print("No memories directory found.")
    return

  md_files = scan_memories(memories_dir)
  if not md_files:
    out.print("No memories found.")
    return

  tag_filters = [t.strip() for t in tags.split(",") if t.strip()] if tags else []

  table = Table(title="Memories")
  table.add_column("ID", style="dim")
  table.add_column("Repo", style="green")
  table.add_column("Type", style="magenta")
  table.add_column("Tags", style="cyan")
  table.add_column("Content", style="white")

  count = 0
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue

    # Apply filters
    if repo and repo not in mem.repo:
      continue
    if type and mem.type != type:
      continue
    if tag_filters and not any(t in mem.tags for t in tag_filters):
      continue

    preview = mem.content[:50] + "..." if len(mem.content) > 50 else mem.content
    table.add_row(
      mem.id,
      ", ".join(mem.repo),
      mem.type,
      ", ".join(mem.tags),
      preview,
    )
    count += 1

  out.print(table)
  out.print(f"\n[bold]{count}[/bold] memories found.")


@app.command()
def delete(
  memory_id: str = typer.Argument(..., help="ID of memory to delete."),
  verbose: bool = verbose_option,
) -> None:
  """Delete a memory by ID."""
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    console.print("[red]Memories directory not found.[/red]")
    raise typer.Exit(1)

  # Find the file via glob
  matches = list(memories_dir.glob(f"{memory_id}.md"))
  if not matches:
    console.print(f"[red]Memory file not found: {memory_id}[/red]")
    raise typer.Exit(1)

  # Delete file
  filepath = matches[0]
  filepath.unlink()

  # Delete from index
  try:
    idx = get_index()
    idx.delete(memory_id)
  except Exception:
    console.print("[yellow]Warning: could not remove from index.[/yellow]")

  git_sync(get_rag_dir(), f"delete: {memory_id}")
  out.print(f"Deleted memory [bold]{memory_id}[/bold]")


@app.command()
def index(
  force: bool = typer.Option(False, "--force", "-f", help="Force full rebuild."),
  verbose: bool = verbose_option,
) -> None:
  """Build or rebuild the vector index."""
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    console.print("[yellow]No memories directory found. Nothing to index.[/yellow]")
    raise typer.Exit(0)

  idx = get_index()
  stats = idx.build(memories_dir, force=force)

  out.print(
    f"Index built: {stats['new']} new, {stats['updated']} updated, {stats['deleted']} deleted."
  )
  out.print(f"Total indexed: {idx.count()}")


@app.command()
def migrate(
  verbose: bool = verbose_option,
) -> None:
  """Auto-migrate memories: flatten nested dirs, rebuild index if model changed.

  Safe to run repeatedly — skips if nothing to do. Called automatically by
  the SessionStart hook on every session.
  """
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    return

  changed = False

  # 1. Flatten nested directories — move .md files to memories/ root
  subdirs = [p for p in memories_dir.iterdir() if p.is_dir()]
  for subdir in subdirs:
    for md_file in subdir.glob("*.md"):
      target = memories_dir / md_file.name
      if target.exists():
        # Duplicate — remove nested copy (flat version is canonical)
        md_file.unlink()
      else:
        md_file.rename(target)
      changed = True
    # Remove empty subdirectory
    if not any(subdir.iterdir()):
      subdir.rmdir()

  if changed:
    out.print(f"Migrated: flattened {len(subdirs)} nested director(ies).")

  # 2. Rebuild index if model changed (build() auto-detects via model_matches)
  idx = get_index()
  if not idx.model_matches():
    out.print("Model changed — rebuilding index...")
    stats = idx.build(memories_dir, force=True)
    out.print(f"Index rebuilt: {stats['new']} new. Total: {idx.count()}")
  elif not idx.schema_matches():
    out.print("Schema version changed — rebuilding index...")
    stats = idx.build(memories_dir, force=True)
    out.print(f"Index rebuilt: {stats['new']} new. Total: {idx.count()}")
  elif changed:
    # Files moved but model didn't change — incremental rebuild
    stats = idx.build(memories_dir)
    out.print(f"Index updated: {stats['new']} new. Total: {idx.count()}")


@app.command()
def context(
  repo: str | None = typer.Option(None, "--repo", "-r", help="Filter by repo."),
  limit: int = typer.Option(10, "--limit", "-n", help="Max non-summary memories."),
  verbose: bool = verbose_option,
) -> None:
  """Show recent memories for context restoration."""
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    out.print("No memories found.")
    return

  md_files = scan_memories(memories_dir)
  if not md_files:
    out.print("No memories found.")
    return

  target_repo = repo or get_repo()

  summaries: list[Memory] = []
  regular: list[Memory] = []
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue
    if target_repo is not None and target_repo not in mem.repo:
      continue
    if mem.type == "session-summary":
      summaries.append(mem)
    else:
      regular.append(mem)

  if not summaries and not regular:
    out.print(f"No memories found for repo: {target_repo}")
    return

  # Section 1: Latest session summary
  if summaries:
    summaries.sort(key=lambda m: m.created, reverse=True)
    latest = summaries[0]
    out.print("### Last Session")
    out.print(latest.content)
    out.print()

  # Section 2: Recent non-summary memories
  if regular:
    regular.sort(key=lambda m: m.created, reverse=True)
    regular = regular[:limit]
    out.print("### Recent Memories")
    for mem in regular:
      preview = mem.content[:120] + "..." if len(mem.content) > 120 else mem.content
      out.print(f"- [{mem.created.isoformat()}] ({mem.type}) {preview}")


@app.command()
def doctor(
  verbose: bool = verbose_option,
) -> None:
  """Check system health: files, index, git context."""
  from qwick_memory.git_utils import detect_author, detect_repo_name
  from qwick_memory.index import MODEL_NAME, MemoryIndex

  ok = True
  memories_dir = get_memories_dir()
  vectordb_dir = get_vectordb_dir()

  # 1. Check memories/ exists
  out.print("[bold]Checking memories directory...[/bold]")
  if memories_dir.exists():
    out.print(f"  memories/ exists at {memories_dir}")
  else:
    console.print("  [red]memories/ not found[/red]")
    ok = False

  # 2. Check memory files validity
  out.print("[bold]Checking memory files...[/bold]")
  if memories_dir.exists():
    md_files = scan_memories(memories_dir)
    valid = 0
    invalid = 0
    for fp in md_files:
      try:
        parse_memory(fp)
        valid += 1
      except Exception as exc:
        console.print(f"  [red]Invalid: {fp.name} — {exc}[/red]")
        invalid += 1
        ok = False
    out.print(f"  {valid} valid, {invalid} invalid memory files")
  else:
    out.print("  Skipped (no memories directory)")

  # 2b. Check for nested directories in memories/
  if memories_dir.exists():
    subdirs = [p for p in memories_dir.iterdir() if p.is_dir()]
    if subdirs:
      console.print(
        f"  [yellow]Found nested directories: {[d.name for d in subdirs]}. "
        f"Flat layout expected — remove them or move files to memories/.[/yellow]"
      )

  # 3. Check .vectordb/ health
  out.print("[bold]Checking vector database...[/bold]")
  if vectordb_dir.exists():
    out.print(f"  .vectordb/ exists at {vectordb_dir}")
    try:
      idx = MemoryIndex(vectordb_dir)
      count = idx.count()
      out.print(f"  Index contains {count} entries")
    except Exception as exc:
      console.print(f"  [red]Index error: {exc}[/red]")
      ok = False
  else:
    console.print("  [yellow].vectordb/ not found (run 'qwick-memory index' to create)[/yellow]")

  # 4. Index consistency
  out.print("[bold]Checking index consistency...[/bold]")
  if memories_dir.exists() and vectordb_dir.exists():
    md_files = scan_memories(memories_dir)
    try:
      idx = MemoryIndex(vectordb_dir)
      index_count = idx.count()
      disk_count = len(md_files)
      if index_count == disk_count:
        out.print(f"  Consistent: {disk_count} files, {index_count} index entries")
      else:
        console.print(
          f"  [yellow]Mismatch: {disk_count} files on disk, "
          f"{index_count} in index. Run 'qwick-memory index' to sync.[/yellow]"
        )
    except Exception:
      out.print("  Skipped (index not available)")
  else:
    out.print("  Skipped (missing memories or vectordb)")

  # 5. Model version
  out.print("[bold]Checking model version...[/bold]")
  if vectordb_dir.exists():
    try:
      idx = MemoryIndex(vectordb_dir)
      if idx.model_matches():
        out.print(f"  Model: {MODEL_NAME}")
      else:
        stored = idx._current_meta.get("model", "unknown")
        console.print(
          f"  [yellow]Model mismatch: index has '{stored}', "
          f"expected '{MODEL_NAME}'. Run 'qwick-memory index --force'.[/yellow]"
        )
    except Exception as exc:
      console.print(f"  [red]Cannot check model version: {exc}[/red]")
      ok = False
  else:
    out.print("  No vectordb found (index not built yet)")

  # 6. Git context
  out.print("[bold]Checking git context...[/bold]")
  try:
    repo_name = detect_repo_name()
    out.print(f"  Repo: {repo_name}")
  except Exception:
    console.print("  [yellow]Could not detect repo name[/yellow]")

  try:
    author = detect_author()
    out.print(f"  Author: {author}")
  except Exception:
    console.print("  [yellow]Could not detect author[/yellow]")

  out.print()
  if ok:
    out.print("[bold green]All checks passed.[/bold green]")
  else:
    console.print("[bold red]Some checks failed. See above.[/bold red]")
    raise typer.Exit(1)
