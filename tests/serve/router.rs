//! The router and its `guard` middleware (loopback Host check + token gate on
//! `/` and `/api/*`) are async glue over private helpers, so they are covered
//! end-to-end by `tests/cli/serve.rs` — which asserts token-absent → 401,
//! non-loopback Host → 403, and the authorized happy paths against a real
//! bound server. This file exists to satisfy the 1:1 `src`↔`tests` mirror.
