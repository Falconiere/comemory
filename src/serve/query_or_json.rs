//! One extractor for a request type a route serves as both `GET` and `POST`.
//!
//! The search-shaped routes (`find`, `search`, `memories/search`, `context`,
//! `code/search`) answer the same request two ways: `GET` carries it in the
//! query string and `POST` in a JSON body (only `POST` can carry a `vector`,
//! since an embedding does not fit in a URL). Their handlers used to be twins
//! that differed in nothing but the extractor. [`QueryOrJson`] makes that the
//! one thing that varies, so a single handler registered as
//! `get(h).post(h)` serves both.

use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::Method;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

/// The request type `T`, read from the JSON body on `POST` and from the query
/// string on every other method.
///
/// Each branch delegates to the stock extractor, and its rejection is the
/// stock rejection's own response, unchanged — status, content type and body
/// — so a client cannot tell this from a handler per method. `HEAD` is routed
/// to the `get` handler by axum and so takes the query-string path, as the
/// `GET` it shadows does.
#[derive(Debug, Clone, Copy)]
pub struct QueryOrJson<T>(pub T);

impl<S, T> FromRequest<S> for QueryOrJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        if req.method() == Method::POST {
            return Json::<T>::from_request(req, state)
                .await
                .map(|Json(value)| Self(value))
                .map_err(IntoResponse::into_response);
        }
        let (mut parts, _body) = req.into_parts();
        Query::<T>::from_request_parts(&mut parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(IntoResponse::into_response)
    }
}

#[cfg(test)]
#[path = "tests/query_or_json.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/query_or_json_routes.rs"]
mod route_tests;
