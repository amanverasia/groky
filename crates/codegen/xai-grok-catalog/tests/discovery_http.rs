//! Integration tests for credential-safe discovery HTTP behavior.

use wiremock::matchers::{header, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};
use xai_grok_catalog::{HttpError, RequestKind, SecretString, get_bounded};

const BEARER: &str = "sk-janus-test";

fn secret() -> SecretString {
    SecretString::new(BEARER)
}

fn client() -> reqwest::Client {
    xai_grok_catalog::http::client()
}

/// Matches only requests that carry no `authorization` header at all.
struct NoAuthorizationHeader;

impl Match for NoAuthorizationHeader {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key("authorization")
    }
}

#[tokio::test]
async fn same_origin_redirect_keeps_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "/v1/models-final"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models-final"))
        .and(header("authorization", format!("Bearer {BEARER}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
        .mount(&server)
        .await;

    let response = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        Some(&secret()),
        false,
        RequestKind::Discovery,
    )
    .await
    .expect("same-origin redirect should succeed with credential intact");
    assert_eq!(response.status.as_u16(), 200);
    assert_eq!(response.body, br#"{"data":[]}"#);
    assert!(response.final_url.path().ends_with("/v1/models-final"));
}

#[tokio::test]
async fn cross_origin_redirect_strips_bearer() {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "location",
            format!("{}/v1/models", target.uri()).as_str(),
        ))
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(NoAuthorizationHeader)
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
        .mount(&target)
        .await;

    let response = get_bounded(
        &client(),
        &format!("{}/v1/models", origin.uri()),
        Some(&secret()),
        false,
        RequestKind::Discovery,
    )
    .await
    .expect("cross-origin redirect should succeed with credential stripped");
    assert_eq!(response.status.as_u16(), 200);
    assert_eq!(response.body, br#"{"data":[]}"#);
}

#[tokio::test]
async fn redirect_loop_stops_at_five() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/v1/models"))
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        Some(&secret()),
        false,
        RequestKind::Discovery,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::TooManyRedirects);
}

#[tokio::test]
async fn redirect_to_disallowed_plain_http_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://192.168.1.20:1/x"),
        )
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        Some(&secret()),
        false,
        RequestKind::Discovery,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::InsecureHttpDenied);
}

#[tokio::test]
async fn content_length_above_limit_is_rejected_before_read() {
    let server = MockServer::start().await;
    let oversize = vec![b'x'; 2 * 1024 * 1024 + 1];
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(oversize))
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        None,
        false,
        RequestKind::Discovery,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::BodyTooLarge);
}

#[tokio::test]
async fn oversize_body_is_body_too_large_not_a_parse_error() {
    // wiremock always sets content-length, so a truly chunked oversize body
    // cannot be simulated here; the streaming cap is still exercised by the
    // accumulator in get_bounded. This test locks the error type for oversize
    // responses regardless of framing.
    let server = MockServer::start().await;
    let oversize = vec![b'{'; 3 * 1024 * 1024];
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(oversize))
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        None,
        false,
        RequestKind::Discovery,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::BodyTooLarge);
}

#[tokio::test]
async fn non_success_status_is_reported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        Some(&secret()),
        false,
        RequestKind::Discovery,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::Status(401));
}

#[tokio::test]
async fn slow_health_check_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"data":[]}"#)
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let err = get_bounded(
        &client(),
        &format!("{}/v1/models", server.uri()),
        None,
        false,
        RequestKind::Health,
    )
    .await
    .unwrap_err();
    assert_eq!(err, HttpError::Timeout);
}
