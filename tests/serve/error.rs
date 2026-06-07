use axum::http::StatusCode;
use axum::response::IntoResponse;
use comemory::errors::Error;
use comemory::serve::error::ApiError;

fn body_string(resp: axum::response::Response) -> (StatusCode, String) {
    let status = resp.status();
    let bytes = futures::executor::block_on(async {
        axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap()
    });
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn not_found_renders_404_envelope() {
    let e = ApiError::not_found("node m:zzzz");
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("\"code\":\"not_found\""));
    assert!(body.contains("node m:zzzz"));
}

#[test]
fn invalid_param_renders_400() {
    let (status, body) = body_string(ApiError::invalid_param("bad layer").into_response());
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("\"code\":\"invalid_param\""));
}

#[test]
fn graph_error_maps_to_500() {
    let e: ApiError = Error::Other("kuzu boom".into()).into();
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("\"code\":\"graph_error\""));
}

#[test]
fn io_error_maps_to_500_io_code() {
    let ioe = std::io::Error::new(std::io::ErrorKind::NotFound, "no file");
    let e: ApiError = Error::Io(ioe).into();
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("\"code\":\"io_error\""));
}
