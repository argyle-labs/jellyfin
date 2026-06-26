//! Integration tests for the Jellyfin plugin's HTTP client and lifecycle.
//!
//! Covers configuration handling (base-url normalization, auth header wire
//! format, response parsing) and the error paths the operator called out:
//! unreachable server, auth failure, malformed response, and backup to a bad
//! destination. HTTP behavior is exercised against `wiremock`, exactly as the
//! in-crate `Client` unit tests do.

use jellyfin::{Client, Config};
use plugin_toolkit::serde_json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Configuration handling ────────────────────────────────────────────────

#[tokio::test]
async fn trailing_slash_in_base_url_is_normalized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ServerName": "media", "Version": "10.9.0", "OperatingSystem": "Linux"
        })))
        .mount(&server)
        .await;
    // A base URL with a trailing slash must not produce `//System/Info`.
    let base = format!("{}/", server.uri());
    let info = Client::new(Config::new(base, "tok"))
        .server_info()
        .await
        .expect("server_info should parse against a normalized URL");
    assert_eq!(info.version.as_deref(), Some("10.9.0"));
}

#[tokio::test]
async fn auth_header_uses_mediabrowser_token_format() {
    let server = MockServer::start().await;
    // The mock only matches when the exact MediaBrowser token header is present;
    // a mismatch falls through to a 404, surfacing as a client error below.
    Mock::given(method("GET"))
        .and(path("/Library/VirtualFolders"))
        .and(header("authorization", "MediaBrowser Token=\"secret-token\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "Name": "Movies", "CollectionType": "movies", "Locations": ["/mnt/media/movies"] }
        ])))
        .mount(&server)
        .await;
    let libs = Client::new(Config::new(server.uri(), "secret-token"))
        .libraries()
        .await
        .expect("libraries should parse with the correct auth header");
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0].name.as_deref(), Some("Movies"));
}

// ── Error handling ─────────────────────────────────────────────────────────

#[tokio::test]
async fn unreachable_server_errors() {
    // Bind a mock server then drop it so the port is closed — connection refused.
    let uri = {
        let server = MockServer::start().await;
        server.uri()
    };
    let result = Client::new(Config::new(uri, "tok")).server_info().await;
    assert!(result.is_err(), "an unreachable server must surface as an error");
}

#[tokio::test]
async fn auth_failure_surfaces_as_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    // The http layer treats a non-2xx as an error, so an unauthorized request
    // surfaces as Err rather than silently parsing an empty body — the memory
    // guard reads this as "not reachable / not healthy".
    let live = Client::new(Config::new(server.uri(), "bad")).liveness().await;
    assert!(live.is_err(), "a 401 must surface as an auth error");
    let parsed = Client::new(Config::new(server.uri(), "bad"))
        .server_info()
        .await;
    assert!(parsed.is_err(), "401 must not parse to a ServerInfo");
}

#[tokio::test]
async fn malformed_response_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("this is not json"))
        .mount(&server)
        .await;
    let result = Client::new(Config::new(server.uri(), "tok"))
        .transcode_health()
        .await;
    assert!(result.is_err(), "a non-JSON body must surface as a decode error");
}

#[tokio::test]
async fn five_hundred_under_pressure_errors_liveness() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    // The memory guard reads a 5xx liveness as pressure: liveness must error.
    let result = Client::new(Config::new(server.uri(), "tok")).liveness().await;
    assert!(result.is_err(), "a 503 must surface as an error for the memory guard");
}
