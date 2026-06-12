//! The request handlers need a live `AppState` (a database connection, the
//! session token, the repo-root map) that only the server bootstrap builds, so
//! they are exercised end-to-end by `tests/cli/serve.rs`: `GET /api/graph`
//! shape, the `GET`→`PUT`→`GET` edit round trip, the stale-`If-Match` 409, and
//! the read-only 405. This file exists to satisfy the 1:1 `src`↔`tests` mirror.
