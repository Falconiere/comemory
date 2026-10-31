//! Declared provenance and vectors for derived document and code embeddings.

use toolu_orm::core::column::{Blob, Integer, Text};
use toolu_orm::table;

/// Local chunk vectors tied to their document revision and provider.
#[table(name = "document_vectors")]
#[primary_key(document_id, ordinal)]
#[foreign_key(
    name = "document_vectors_chunk_fk",
    columns(document_id, ordinal),
    references = "document_chunks(document_id, ordinal)",
    on_delete = "cascade"
)]
pub struct DocumentVectors {
    /// Document identity.
    #[column(not_null)]
    pub document_id: Text,
    /// Chunk position.
    #[column(not_null)]
    pub ordinal: Integer,
    /// Provider configuration digest.
    #[column(not_null)]
    pub model: Text,
    /// Encoded 1024-dimensional vector.
    #[column(not_null)]
    pub embedding: Blob,
}

/// Active provider generation for one source.
#[table(name = "document_vector_source")]
pub struct DocumentVectorSource {
    /// Owning source.
    #[column(
        primary_key,
        not_null,
        references = "source_roots(id)",
        on_delete = "cascade"
    )]
    pub source_id: Text,
    /// Active provider identity.
    #[column(not_null)]
    pub model: Text,
    /// Unique run generation.
    #[column(not_null)]
    pub generation: Text,
}

/// Provider identity of automatically generated code vectors.
#[table(name = "code_vector_provenance")]
pub struct CodeVectorProvenance {
    /// Owning code symbol.
    #[column(primary_key, references = "code_symbols(id)", on_delete = "cascade")]
    pub symbol_id: Integer,
    /// Provider configuration digest.
    #[column(not_null)]
    pub model: Text,
}
