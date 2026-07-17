//! End-to-end proof of exact model IDs and credential flow for dynamic
//! (Janus) providers — plan 3 Task 11.
//!
//! Two stack levels are covered:
//!
//! 1. **Handler + real sampling path** (default `cargo test`): drives the
//!    real ACP handler fn (`extensions::providers::setup_janus`) against a
//!    `ProviderCatalogAdapter` + `ModelsManager`, switches the manager's
//!    current model to the composed picker key, builds the `SamplerConfig`
//!    through `ModelsManager::sampling_config()` (the production credential
//!    seam), and issues the chat completion through the real
//!    `SamplingClient`. Proves the mock saw health + discovery + inference
//!    all with the stored bearer, and that the inference body carries the
//!    exact upstream model ID (no `janus/` prefix).
//! 2. **Full ACP stdio stack** (`#[ignore]`, requires the pre-built binary,
//!    run with `--ignored`): spawns `grok agent stdio`, calls
//!    `x.ai/providers/setup_janus` over the wire, switches the session model
//!    via `session/set_model`, prompts, and asserts the same wire contract
//!    against a second (Janus) mock server.

use std::sync::Arc;

use agent_client_protocol as acp;
use futures_util::StreamExt as _;
use serial_test::serial;
use xai_grok_shell::agent::config::Config;
use xai_grok_shell::agent::models::ModelsManager;
use xai_grok_shell::agent::provider_catalog::ProviderCatalogAdapter;
use xai_grok_shell::auth::{AuthManager, GrokComConfig};
use xai_grok_shell::extensions::providers::{ProviderSurface, SetupJanusRequest, setup_janus};
use xai_grok_shell::sampling::{Client, ConversationItem, ConversationRequest};
use xai_grok_test_support::{GrokStdioClient, MockInferenceServer, MockModelEntry, git_workdir};

const JANUS_KEY: &str = "sk-janus-e2e";

fn janus_models() -> Vec<MockModelEntry> {
    vec![
        MockModelEntry::new("openai/gpt-4o"),
        MockModelEntry::new("best-effort"),
    ]
}

/// Fixture mirroring `tests/provider_janus_setup.rs`: real ModelsManager +
/// adapter over a temp `$GROK_HOME`.
fn surface(tmp: &std::path::Path) -> ProviderSurface {
    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(tmp.to_path_buf()));
    let auth = Arc::new(AuthManager::new(tmp, GrokComConfig::default()));
    let mgr = ModelsManager::from_config(&Config::default(), None, auth).unwrap();
    mgr.set_provider_catalog(adapter);
    ProviderSurface {
        models_manager: mgr,
        grok_home: tmp.to_path_buf(),
        xai_configured: false,
    }
}

fn setup_request(base_url: &str, api_key: Option<&str>) -> SetupJanusRequest {
    SetupJanusRequest {
        base_url: base_url.to_owned(),
        api_key: api_key.map(str::to_owned),
        allow_insecure_http: false,
    }
}

/// Run one chat completion through the real `SamplingClient` and return the
/// concatenated assistant text.
async fn collect_assistant_text(
    sampling: xai_grok_shell::sampling::SamplerConfig,
    prompt: &str,
) -> String {
    let client = Client::new(sampling).expect("build sampling client");
    let request = ConversationRequest::from_items(vec![ConversationItem::user(prompt)]);
    let (mut stream, _metadata) = client
        .conversation_stream(request)
        .await
        .expect("chat completion stream must open");
    let mut content = String::new();
    while let Some(chunk) = stream.next().await {
        for choice in chunk.expect("stream chunk").choices {
            if let Some(text) = choice.delta.content {
                content.push_str(&text);
            }
        }
    }
    content
}

/// Assert every leg of the wire contract for a completed Janus round-trip:
/// health + discovery + inference all carried the stored bearer, and the
/// inference body used exactly `expected_upstream_id`.
fn assert_wire_contract(server: &MockInferenceServer, expected_upstream_id: &str) {
    let requests = server.requests();
    let log = server.request_log_summary();

    let health = requests
        .iter()
        .find(|r| r.method == "GET" && r.path == "/v1/health")
        .unwrap_or_else(|| panic!("setup must GET /v1/health\nrequest log:\n{log}"));
    assert_eq!(
        health.authorization.as_deref(),
        Some(&*format!("Bearer {JANUS_KEY}")),
        "health probe must carry the stored bearer\nrequest log:\n{log}"
    );

    let models = requests
        .iter()
        .find(|r| r.method == "GET" && r.path == "/v1/models")
        .unwrap_or_else(|| panic!("setup must GET /v1/models\nrequest log:\n{log}"));
    assert_eq!(
        models.authorization.as_deref(),
        Some(&*format!("Bearer {JANUS_KEY}")),
        "model discovery must carry the stored bearer\nrequest log:\n{log}"
    );

    let inference = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/chat/completions")
        .unwrap_or_else(|| panic!("prompt must POST /v1/chat/completions\nrequest log:\n{log}"));
    assert_eq!(
        inference.authorization.as_deref(),
        Some(&*format!("Bearer {JANUS_KEY}")),
        "inference must carry the stored bearer\nrequest log:\n{log}"
    );
    let body = inference
        .body
        .as_ref()
        .expect("chat completion request has a body");
    assert_eq!(
        body["model"].as_str(),
        Some(expected_upstream_id),
        "inference body must carry the exact upstream model ID \
         (no janus/ prefix)\nrequest log:\n{log}"
    );
}

/// One `$GROK_HOME` shared by the handler-level tests in this binary.
///
/// `crate::util::grok_home::grok_home()` memoizes its first resolution for
/// the whole process, and `ModelsManager::sampling_config()` resolves the
/// stored provider key through it — so every `#[serial(provider_env)]` test
/// that exercises the production credential seam must point `GROK_HOME` at
/// the same directory. Each test still gets a fresh adapter/manager and a
/// fresh mock server; `setup_janus` re-registers Janus with the new base
/// URL, so state files in the shared home never leak stale endpoints.
fn shared_grok_home() -> &'static std::path::Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| tempfile::tempdir().expect("create shared GROK_HOME"))
        .path()
}

/// Drive setup → model switch → inference through the handler + real
/// sampling-config path for one picker key / upstream ID pair.
async fn run_handler_level_roundtrip(picker_key: &str, upstream_id: &str) {
    let home = shared_grok_home();
    unsafe { std::env::set_var("GROK_HOME", home) };

    let server = MockInferenceServer::start_with_required_auth(janus_models(), JANUS_KEY)
        .await
        .unwrap();
    server.set_response("Janus answer");

    // Setup through the real ACP handler (persists config + key,
    // health-checks, discovers, rebuilds the composed model catalog).
    let surface = surface(home);
    let response = setup_janus(&surface, setup_request(&server.url(), Some(JANUS_KEY)))
        .await
        .expect("setup_janus handler");
    let response_json = serde_json::to_string(&response).unwrap();
    assert!(
        response_json.contains("\"state\":\"ready\"") && response_json.contains("\"modelCount\":2"),
        "setup must be ready with 2 models: {response_json}\nrequest log:\n{}",
        server.request_log_summary()
    );

    // The composed catalog exposes the qualified picker key; switching the
    // manager's current model must resolve to the dynamic provider entry.
    let mgr = &surface.models_manager;
    let models = mgr.models();
    assert!(
        models.contains_key(picker_key),
        "composed models must contain {picker_key}; got: {:?}",
        models.keys().collect::<Vec<_>>()
    );
    mgr.set_current_model_id(acp::ModelId::new(picker_key));

    // Production credential seam: sampling_config() resolves the stored
    // provider key and the dynamic base_url; nothing is handcrafted.
    let sampling = mgr.sampling_config();
    assert_eq!(
        sampling.model, upstream_id,
        "SamplerConfig.model must be the exact upstream ID"
    );
    assert_eq!(sampling.base_url, server.url());
    assert_eq!(
        sampling.api_key.as_deref(),
        Some(JANUS_KEY),
        "the stored Janus key must be injected at SamplerConfig construction"
    );

    let content = collect_assistant_text(sampling, "hello Janus").await;
    assert_eq!(
        content,
        "Janus answer",
        "request log:\n{}",
        server.request_log_summary()
    );

    assert_wire_contract(&server, upstream_id);

    unsafe { std::env::remove_var("GROK_HOME") };
}

/// Slash-carrying upstream ID: discovered `openai/gpt-4o` composes to picker
/// key `janus/openai/gpt-4o` and infers with body model `openai/gpt-4o`.
#[tokio::test(flavor = "multi_thread")]
#[serial(provider_env)]
async fn janus_discovers_slash_id_and_infers_with_same_id_and_bearer() {
    run_handler_level_roundtrip("janus/openai/gpt-4o", "openai/gpt-4o").await;
}

/// Combo ID without a slash: `best-effort` composes to `janus/best-effort`
/// and the inference body preserves `best-effort` exactly.
#[tokio::test(flavor = "multi_thread")]
#[serial(provider_env)]
async fn janus_combo_id_best_effort_is_preserved_on_the_wire() {
    run_handler_level_roundtrip("janus/best-effort", "best-effort").await;
}

/// Full ACP stdio stack: a real `grok agent stdio` process, setup via the
/// `x.ai/providers/setup_janus` extension, model switch via
/// `session/set_model`, and a prompt that must stream the Janus mock's
/// fixed answer. The Janus mock is a second server (with required auth) so
/// the assertion also proves inference left the xAI mainline entirely.
#[tokio::test]
#[ignore] // requires pre-built binary; run with --ignored
async fn full_acp_janus_setup_switch_and_prompt_hits_exact_upstream_id() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let workdir = git_workdir();
            // Mainline mock: unauthenticated, one plain model.
            let mainline = MockInferenceServer::start().await.unwrap();
            // Janus mock: bearer-gated, two models, fixed answer.
            let janus = MockInferenceServer::start_with_required_auth(janus_models(), JANUS_KEY)
                .await
                .unwrap();
            janus.set_response("Janus answer");

            let client = GrokStdioClient::spawn(&mainline, workdir.path()).await;
            client.initialize_with_timeout().await;
            let session_id = client.create_session_with_timeout(workdir.path()).await;

            let response = client
                .ext_method(
                    "x.ai/providers/setup_janus",
                    serde_json::json!({
                        "baseUrl": janus.url(),
                        "apiKey": JANUS_KEY,
                        "allowInsecureHttp": false,
                    }),
                )
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "setup_janus ext failed: {e:?}\nstderr:\n{}\njanus log:\n{}",
                        client.stderr(),
                        janus.request_log_summary()
                    )
                });
            let response: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
            assert_eq!(
                response["state"],
                "ready",
                "janus setup over ACP must be ready: {response}\njanus log:\n{}",
                janus.request_log_summary()
            );
            assert_eq!(response["modelCount"], 2, "{response}");

            client
                .set_model_with_timeout(&session_id, "janus/openai/gpt-4o")
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "set_model(janus/openai/gpt-4o) failed: {e:?}\nstderr:\n{}",
                        client.stderr()
                    )
                });

            client
                .prompt_with_timeout(&session_id, "hello Janus")
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "prompt failed: {e:?}\nstderr:\n{}\njanus log:\n{}",
                        client.stderr(),
                        janus.request_log_summary()
                    )
                });
            assert!(
                client.captured_text().contains("Janus answer"),
                "prompt must stream the Janus mock's fixed answer; got: {:?}\njanus log:\n{}",
                client.captured_text(),
                janus.request_log_summary()
            );

            assert_wire_contract(&janus, "openai/gpt-4o");
        })
        .await;
}
