//! The single `Error` → HTTP status mapping behind the API.

use axum::response::IntoResponse;
use comemory::errors::Error;
use comemory::serve::error::ApiError;

fn status_of(e: Error) -> u16 {
    ApiError(e).into_response().status().as_u16()
}

#[test]
fn maps_errors_to_expected_status_codes() {
    assert_eq!(status_of(Error::NotFound("x".into())), 404);
    assert_eq!(status_of(Error::Forbidden("x".into())), 403);
    assert_eq!(status_of(Error::BadRequest("x".into())), 400);
    assert_eq!(status_of(Error::Other("x".into())), 500);
}

#[test]
fn missing_file_io_error_is_404() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
    assert_eq!(status_of(Error::Io(io)), 404);
}

#[test]
fn other_io_error_is_500() {
    let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
    assert_eq!(status_of(Error::Io(io)), 500);
}
