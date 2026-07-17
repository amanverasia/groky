//! PTY e2e: guided Janus provider setup through `/providers`, then a model
//! switch and a real inference round-trip against the Janus mock
//! (plan 3 Task 11).
//!
//! Ignored for ordinary Cargo runs like every PTY family; run with
//! `cargo test -p xai-grok-pager --test pty_e2e_provider_setup -- --ignored`.

// Shared support intentionally serves all PTY family crates.
#[allow(dead_code, unused_imports)]
#[path = "pty_e2e/common.rs"]
mod common;

use common::*;
use xai_grok_test_support::mock_server::MockInferenceServer;

/// Fixed response sentinel streamed by the Janus mock only.
const JANUS_SENTINEL: &str = "JANUSFIXEDRESPONSE";

/// Press one key and give the event loop a beat to process it.
fn tap(harness: &mut PtyHarness, bytes: &[u8], settle_ms: u64) {
    harness.inject_keys(bytes).expect("inject key");
    harness.update(Duration::from_millis(settle_ms));
}

/// `wait_for_text` with the Janus mock's request log in the failure message.
fn wait_or_dump(
    harness: &mut PtyHarness,
    janus: &MockInferenceServer,
    needle: &str,
    timeout: Duration,
    what: &str,
) {
    harness.wait_for_text(needle, timeout).unwrap_or_else(|e| {
        panic!(
            "{what}: {e}\nscreen:\n{}\njanus request log:\n{}",
            harness.screen_contents(),
            janus.request_log_summary()
        )
    });
}

/// **Janus setup → model switch → inference, through the real TUI.**
///
/// 1. Mainline mock (`test-model`) drives the pager/agent; a second mock
///    (`openai/gpt-4o`, `best-effort`) plays Janus.
/// 2. `/providers` → select the Janus row → replace the prefilled base URL
///    with the Janus mock URL (loopback http, no confirmation) → skip the
///    optional key → `Janus is ready. 2 models available.`
/// 3. `/model janus/openai/gpt-4o` switches to the discovered model.
/// 4. A prompt streams the Janus mock's fixed response, and the recorded
///    inference body carries the exact upstream ID `openai/gpt-4o`.
///
/// Not covered (noted in the task report): dropping the Janus server and
/// force-refreshing to observe cached-models messaging — reopening the
/// providers modal mid-session and racing the background refresh proved too
/// timing-sensitive for a deterministic PTY assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn janus_setup_switch_and_prompt_renders_janus_response() {
    let content = ContentController::start().await.expect("start content");
    content.set_response(format!("{MOCK_RESPONSE_SENTINEL} mainline turn."));

    let janus = MockInferenceServer::start_with_models(vec![
        MockModel::new("openai/gpt-4o"),
        MockModel::new("best-effort"),
    ])
    .await
    .expect("start janus mock");
    janus.set_response(format!("{JANUS_SENTINEL} hello from janus."));

    let binary = pager_binary().expect("resolve pager binary");
    let mut harness =
        PtyHarness::spawn_with_content(&binary, DEFAULT_ROWS, DEFAULT_COLS, &content, &[])
            .expect("spawn pager");

    harness
        .wait_for_text(WELCOME_SCREEN_SENTINEL, WELCOME_TIMEOUT)
        .expect("welcome text");

    // ── /providers → Janus row ──────────────────────────────────────────
    // Typing `/` promotes the welcome screen into a real session first.
    inject_keys_paced(&mut harness, b"/providers\r");
    // The Janus setup row is appended last, far below the ~150 catalog
    // providers, so it is not visible until the selection scrolls down:
    // first wait for the list itself (any loaded row / the key hints).
    wait_or_dump(
        &mut harness,
        &janus,
        "Enter select",
        Duration::from_secs(15),
        "providers modal must open with its row list",
    );

    // Down saturates on the last row — the Janus setup row. The embedded
    // catalog carries 150+ providers and the list renders without a scroll
    // offset (rows beyond the modal stay off-screen), so the selection is
    // driven blind and verified by the screen that Enter opens. Arrows are
    // sent in small batches (escape sequences parse as discrete key events
    // and are never paste-coalesced).
    for _ in 0..40 {
        harness
            .inject_keys(&keys::DOWN.repeat(5))
            .expect("inject Down batch");
        harness.update(Duration::from_millis(30));
    }
    tap(&mut harness, keys::ENTER, 200);

    // ── Base URL screen: clear the prefilled default, type the mock URL ─
    // This doubles as the proof that the blind Down run landed on the
    // Janus row: any other row would render its key-entry screen instead.
    wait_or_dump(
        &mut harness,
        &janus,
        "Set up Janus (local): base URL",
        Duration::from_secs(10),
        "Enter on the trailing row must open the Janus setup flow",
    );
    tap(&mut harness, b"\x15", 100); // Ctrl+U clears the input
    inject_keys_paced(&mut harness, janus.url().as_bytes());
    // Loopback http advances without an insecure-URL confirmation.
    tap(&mut harness, keys::ENTER, 200);

    // ── Optional key screen: skip (Enter with an empty buffer) ──────────
    wait_or_dump(
        &mut harness,
        &janus,
        "Optional API key",
        Duration::from_secs(10),
        "key screen must follow the base URL",
    );
    tap(&mut harness, keys::ENTER, 200);

    // ── Result screen ────────────────────────────────────────────────────
    wait_or_dump(
        &mut harness,
        &janus,
        "Janus is ready. 2 models available.",
        Duration::from_secs(10),
        "setup must finish ready with both discovered models",
    );

    // Esc: result → provider list; Esc: close the modal.
    tap(&mut harness, keys::ESC, 200);
    tap(&mut harness, keys::ESC, 300);

    // ── Switch to the discovered model by its qualified picker key ──────
    inject_keys_paced(&mut harness, b"/model janus/openai/gpt-4o\r");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        harness.update(Duration::from_millis(100));
        // The status bar shows the display name: bundled-catalog enrichment
        // (exact full-ID match on `openai/gpt-4o`) renders "GPT-4o", so
        // match case-insensitively.
        let screen_lower = harness.screen_contents().to_lowercase();
        if !screen_lower.contains("/model janus") && screen_lower.contains("gpt-4o") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the Janus model switch\nscreen:\n{}\njanus request log:\n{}",
            harness.screen_contents(),
            janus.request_log_summary()
        );
    }
    assert!(
        !harness.contains_text("Unknown model"),
        "model switch must resolve janus/openai/gpt-4o\nscreen:\n{}",
        harness.screen_contents()
    );

    // ── Prompt: the reply must come from the Janus mock ─────────────────
    inject_keys_paced(&mut harness, b"hello\r");
    wait_or_dump(
        &mut harness,
        &janus,
        JANUS_SENTINEL,
        Duration::from_secs(30),
        "prompt must stream the Janus mock's fixed response",
    );

    // The Janus mock's inference log carries the exact upstream model ID.
    let inference = janus
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == "/v1/chat/completions")
        .unwrap_or_else(|| {
            panic!(
                "janus mock must receive the chat completion\njanus request log:\n{}",
                janus.request_log_summary()
            )
        });
    let body = inference.body.expect("chat completion body");
    assert_eq!(
        body["model"].as_str(),
        Some("openai/gpt-4o"),
        "inference body must carry the exact upstream ID (no janus/ prefix)\n\
         janus request log:\n{}",
        janus.request_log_summary()
    );

    assert!(
        harness.is_running(),
        "pager exited\nscreen:\n{}",
        harness.screen_contents()
    );
    harness.quit().expect("clean quit");
}
