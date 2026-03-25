"""LanceDB-backed vector index for qwick-memory memories."""

from __future__ import annotations

import contextlib
import json
import logging
from typing import TYPE_CHECKING, Any

import lancedb
from fastembed import TextEmbedding

from qwick_memory.memory import Memory, parse_memory, scan_memories

if TYPE_CHECKING:
  from pathlib import Path

logger = logging.getLogger(__name__)

TABLE_NAME = "memories"
MODEL_NAME = "nomic-ai/nomic-embed-text-v1.5-Q"
EXPECTED_DIM = 768
SCHEMA_VERSION = 2


def _enrich_text(memory: Memory) -> str:
  """Prepend metadata to content for richer embeddings and FTS."""
  parts = [f"[Repository: {', '.join(memory.repo)}]"]
  parts.append(f"[Type: {memory.type}]")
  if memory.tags:
    parts.append(f"[Tags: {', '.join(memory.tags)}]")
  return f"{' '.join(parts)}\n{memory.content}"


class MemoryIndex:
  """Manages a LanceDB vector index of Memory objects."""

  def __init__(self, vectordb_dir: Path) -> None:
    self._vectordb_dir = vectordb_dir
    self._vectordb_dir.mkdir(parents=True, exist_ok=True)
    self._db = lancedb.connect(str(vectordb_dir))
    self._model: TextEmbedding | None = None
    self._current_meta = self._read_meta()

  # -- properties ----------------------------------------------------------

  @property
  def model(self) -> TextEmbedding:
    """Lazy-load the embedding model (first call downloads ~130 MB)."""
    if self._model is None:
      self._model = TextEmbedding(MODEL_NAME)
    return self._model

  # -- private helpers -----------------------------------------------------

  def _read_meta(self) -> dict[str, str]:
    """Read meta.json if it exists, return empty dict otherwise."""
    meta_path = self._vectordb_dir / "meta.json"
    if meta_path.exists():
      return json.loads(meta_path.read_text())
    return {}

  def _write_meta(self) -> None:
    """Write meta.json with model name for version tracking."""
    meta_path = self._vectordb_dir / "meta.json"
    meta_path.write_text(json.dumps({"model": MODEL_NAME, "schema_version": SCHEMA_VERSION}))

  # -- embedding helpers ----------------------------------------------------

  def model_matches(self) -> bool:
    """Check if the indexed model matches the current MODEL_NAME."""
    return self._current_meta.get("model") == MODEL_NAME

  def schema_matches(self) -> bool:
    """Check if the indexed schema version matches the current SCHEMA_VERSION."""
    return self._current_meta.get("schema_version") == SCHEMA_VERSION

  def _validate_dim(self, vector: list[float], context: str) -> None:
    """Raise if vector dimension doesn't match expected model output."""
    if len(vector) != EXPECTED_DIM:
      raise RuntimeError(
        f"Embedding dim mismatch in {context}: got {len(vector)}, expected {EXPECTED_DIM}. "
        f"The fastembed model may have fallen back to a different model. "
        f"Try: rm -rf /tmp/fastembed_cache && qwick-memory index --force"
      )

  def _embed_documents(self, texts: list[str]) -> list[list[float]]:
    """Embed documents with 'search_document: ' prefix for nomic model."""
    if not texts:
      return []
    prefixed = [f"search_document: {t}" for t in texts]
    vectors = [vec.tolist() for vec in self.model.embed(prefixed)]
    if vectors:
      self._validate_dim(vectors[0], "embed_documents")
    return vectors

  def _embed_query(self, text: str) -> list[float]:
    """Embed a single query with 'search_query: ' prefix for nomic model."""
    prefixed = f"search_query: {text}"
    vector = next(iter(self.model.embed([prefixed]))).tolist()
    self._validate_dim(vector, "embed_query")
    return vector

  def _table_exists(self) -> bool:
    """Check whether the memories table already exists."""
    result = self._db.list_tables()
    # lancedb 0.30 returns a ListTablesResponse object with a .tables attr
    raw = result.tables if hasattr(result, "tables") else list(result)
    tables: list[str] = [str(t) for t in raw]
    return TABLE_NAME in tables

  def _get_table(self) -> Any | None:
    """Return the existing table, or None."""
    if not self._table_exists():
      return None
    return self._db.open_table(TABLE_NAME)

  def _create_table(self, records: list[dict[str, Any]]) -> Any:
    """Create the memories table and build an FTS index on content."""
    if self._table_exists():
      self._db.drop_table(TABLE_NAME)
    table = self._db.create_table(TABLE_NAME, records)
    try:
      table.create_fts_index("enriched_content", replace=True)
    except Exception:
      logger.warning("Could not create FTS index; full-text search unavailable.")
    return table

  def _memory_to_record(self, memory: Memory, vector: list[float]) -> dict[str, Any]:
    """Convert a Memory + its embedding vector into a flat dict for LanceDB."""
    return {
      "id": memory.id,
      "repo": ",".join(memory.repo),
      "type": memory.type,
      "tags": ",".join(memory.tags),
      "author": memory.author,
      "created": memory.created.isoformat(),
      "content": memory.content,
      "enriched_content": _enrich_text(memory),
      "quality": memory.quality,
      "content_hash": memory.content_hash,
      "vector": vector,
    }

  # -- public API ----------------------------------------------------------

  def upsert(self, memory: Memory) -> None:
    """Insert or update a single memory in the index."""
    vectors = self._embed_documents([_enrich_text(memory)])
    record = self._memory_to_record(memory, vectors[0])

    table = self._get_table()
    if table is None:
      self._create_table([record])
      return

    # Delete existing row with same id, then add new one
    with contextlib.suppress(Exception):
      table.delete(f'id = "{memory.id}"')
    table.add([record])

    with contextlib.suppress(Exception):
      table.optimize()

  def delete(self, memory_id: str) -> None:
    """Delete a memory by its ID."""
    table = self._get_table()
    if table is None:
      return
    try:
      table.delete(f'id = "{memory_id}"')
    except Exception:
      logger.warning("Failed to delete memory %s", memory_id)

  def count(self) -> int:
    """Return the number of rows in the index."""
    table = self._get_table()
    if table is None:
      return 0
    return table.count_rows()

  def build(
    self,
    memories_dir: Path,
    force: bool = False,
  ) -> dict[str, int]:
    """Full or incremental index build.

    Returns a dict with keys: new, updated, deleted.
    """
    md_files = scan_memories(memories_dir)

    # Parse all memories from disk
    disk_memories: dict[str, tuple[Memory, Path]] = {}
    for fp in md_files:
      try:
        mem = parse_memory(fp)
        disk_memories[mem.id] = (mem, fp)
      except Exception:
        logger.warning("Skipping unparseable file: %s", fp)

    # Auto-force rebuild if model has changed
    if not self.model_matches() and not force:
      logger.info(
        "Model changed (%s → %s). Forcing full rebuild.",
        self._current_meta.get("model", "none"),
        MODEL_NAME,
      )
      force = True

    if not self.schema_matches() and not force:
      logger.info(
        "Schema version changed (%s → %s). Forcing full rebuild.",
        self._current_meta.get("schema_version", 1),
        SCHEMA_VERSION,
      )
      force = True

    # Force rebuild: drop and recreate
    if force or not self._table_exists():
      result = self._full_build(disk_memories)
    else:
      result = self._incremental_build(disk_memories)

    self._write_meta()
    self._current_meta = {"model": MODEL_NAME, "schema_version": SCHEMA_VERSION}
    return result

  def _full_build(
    self,
    disk_memories: dict[str, tuple[Memory, Path]],
  ) -> dict[str, int]:
    """Drop existing table and rebuild from scratch."""
    if not disk_memories:
      # Nothing to index -- drop table if it exists
      if self._table_exists():
        self._db.drop_table(TABLE_NAME)
      return {"new": 0, "updated": 0, "deleted": 0}

    memories = [mem for mem, _fp in disk_memories.values()]
    texts = [_enrich_text(mem) for mem in memories]
    vectors = self._embed_documents(texts)

    records = [self._memory_to_record(mem, vec) for mem, vec in zip(memories, vectors, strict=True)]
    self._create_table(records)
    return {"new": len(records), "updated": 0, "deleted": 0}

  def _incremental_build(
    self,
    disk_memories: dict[str, tuple[Memory, Path]],
  ) -> dict[str, int]:
    """Compare disk state to index and apply deltas."""
    table = self._get_table()
    assert table is not None  # guaranteed by caller

    existing_rows = table.to_arrow().to_pylist()
    existing_by_id: dict[str, dict[str, Any]] = {row["id"]: row for row in existing_rows}

    new_ids: list[str] = []
    updated_ids: list[str] = []
    deleted_ids: list[str] = []

    # Detect new and updated
    for mem_id, (mem, _fp) in disk_memories.items():
      if mem_id not in existing_by_id:
        new_ids.append(mem_id)
      elif existing_by_id[mem_id]["content_hash"] != mem.content_hash:
        updated_ids.append(mem_id)

    # Detect deleted
    for existing_id in existing_by_id:
      if existing_id not in disk_memories:
        deleted_ids.append(existing_id)

    changes = len(new_ids) + len(updated_ids) + len(deleted_ids)
    if changes == 0:
      return {"new": 0, "updated": 0, "deleted": 0}

    # Apply deletions
    for did in deleted_ids:
      try:
        table.delete(f'id = "{did}"')
      except Exception:
        logger.warning("Failed to delete %s during incremental build", did)

    # Apply updates (delete + add)
    for uid in updated_ids:
      mem, _fp = disk_memories[uid]
      vec = self._embed_documents([_enrich_text(mem)])[0]
      record = self._memory_to_record(mem, vec)
      with contextlib.suppress(Exception):
        table.delete(f'id = "{uid}"')
      table.add([record])

    # Apply additions
    if new_ids:
      new_memories = [disk_memories[nid][0] for nid in new_ids]
      texts = [_enrich_text(m) for m in new_memories]
      vectors = self._embed_documents(texts)
      records = [
        self._memory_to_record(mem, vec) for mem, vec in zip(new_memories, vectors, strict=True)
      ]
      table.add(records)

    # Rebuild FTS index
    with contextlib.suppress(Exception):
      table.create_fts_index("enriched_content", replace=True)

    # Best-effort optimize
    with contextlib.suppress(Exception):
      table.optimize()

    return {
      "new": len(new_ids),
      "updated": len(updated_ids),
      "deleted": len(deleted_ids),
    }
