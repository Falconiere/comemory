CREATE TABLE document_vectors (
  document_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  model TEXT NOT NULL,
  embedding BLOB NOT NULL,
  PRIMARY KEY (document_id, ordinal),
  FOREIGN KEY (document_id, ordinal) REFERENCES document_chunks(document_id, ordinal) ON DELETE CASCADE
);

CREATE TABLE document_vector_source (
  source_id TEXT PRIMARY KEY NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
  model TEXT NOT NULL,
  generation TEXT NOT NULL
);
