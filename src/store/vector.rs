//! Insert and query helpers around the `memory_vec` and `code_vec`
//! `sqlite-vec` virtual tables.
//!
//! Skeleton committed by Task 2 of the v0.2 plan so downstream tasks
//! can import `crate::store::vector`. Task 5 fills in `insert_memory`,
//! `knn_memory`, `insert_code`, `knn_code`, plus the `dim_memory` /
//! `dim_code` accessors that read from `schema_meta`.
