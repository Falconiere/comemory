//! Provider selection for a unified query.

use crate::config::Config;
use crate::domains::retrieval::find::Request;
use crate::domains::retrieval::scope::{Domain, Domains};
use crate::prelude::*;

pub(super) struct QueryEmbeddings {
    pub memory: Option<Vec<f32>>,
    pub document: Option<Vec<f32>>,
    pub document_model: Option<String>,
}

pub(super) fn for_find(cfg: &Config, req: &Request, domains: Domains) -> Result<QueryEmbeddings> {
    if let Some(vector) = req.vector.as_deref() {
        if vector.len() != cfg.retrieval.memory_vector_dim
            && vector.len() != cfg.retrieval.code_vector_dim
        {
            return Err(Error::Config(format!(
                "find vector must have {} memory or {} code dimensions, got {}",
                cfg.retrieval.memory_vector_dim,
                cfg.retrieval.code_vector_dim,
                vector.len()
            )));
        }
        crate::store::embed::guard_dim(vector, vector.len())?;
    }
    let memory = if domains.contains(Domain::Memory) {
        super::pipeline::query_embedding(cfg, &req.query, req.vector.is_some())?
    } else {
        None
    };
    let provider = if req.vector.is_none() && domains.contains(Domain::Document) {
        cfg.document_embed_cmd.as_deref()
    } else {
        None
    };
    let document_model = provider.map(|cmd| {
        crate::domains::documents::chunk_embedding::provider_id(
            cmd,
            cfg.document_embed_hint.as_deref(),
        )
    });
    let document = if let Some(cmd) = provider {
        if cfg.embed_cmd.as_deref() == Some(cmd)
            && cfg.embed_hint == cfg.document_embed_hint
            && memory.is_some()
        {
            memory.clone()
        } else {
            let vector = crate::utilities::embed::embed_query(cmd, &req.query)?;
            crate::store::embed::guard_dim(&vector, 1024)?;
            Some(vector)
        }
    } else {
        None
    };
    Ok(QueryEmbeddings {
        memory,
        document,
        document_model,
    })
}
