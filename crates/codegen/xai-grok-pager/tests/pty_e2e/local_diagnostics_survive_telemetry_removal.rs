// Per-test-case module for the `pty_e2e` integration test crates.
#[allow(unused_imports)]
use super::common::*;

/// **Local diagnostics survive telemetry removal.**
///
/// The telemetry-removal work deletes remote upload paths but must preserve
/// the local, on-disk diagnostics. This case runs one real prompt against the
/// mock inference server and asserts afterwards that under the isolated
/// `$GROK_HOME`:
///
/// - `logs/unified.jsonl` exists and carries the pager startup entry
///   (`unified_log` — pager forwards over ACP, the shell agent writes),
/// - `logs/sampling.jsonl` exists and carries the agent's
///   `target: "sampling_log"` prompt event (enabled via
///   `GROK_LOG_SAMPLING=true`, the env behind `--log-sampling`; the earlier
///   initialize event can fire before the TUI subscriber is installed, so the
///   prompt event is the deterministic marker),
/// - `debug/` holds at least one non-empty firehose file (enabled via
///   `GROK_DEBUG_LOG=1`, per-session routing) — asserted via a SECOND pager
///   run without sampling, because the sampling `TargetFilterLayer`
///   implements a global `Layer::enabled` (pre-existing, also on `main`)
///   that suppresses every non-`sampling_log` tracing event process-wide,
///   starving the firehose whenever sampling is on.
///
/// Deliberately not asserted: `logs/hooks.log` (opt-in and only written when
/// hook/plugin activity fires — no hook fixture in this harness) and
/// `logs/memory.log` (compiled out unless the `memory-log` feature is on).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn local_diagnostics_survive_telemetry_removal() {
    let content = ContentController::start().await.expect("start content");
    content.set_response(format!("{MOCK_RESPONSE_SENTINEL} diagnostics turn."));
    let binary = pager_binary().expect("resolve pager binary");
    let grok_home = content.home().join(".grok");

    // ── Run 1: sampling on → unified.jsonl + sampling.jsonl ────────────────
    //
    // `env_for_pager` pins HOME + GROK_HOME (= <tmp>/.grok) at the isolated
    // temp dir; add the sampling opt-in on top. GROK_LOG_SAMPLING is the env
    // behind the hidden `--log-sampling` flag (cli.rs) and, unlike the flag,
    // is inherited by any spawned agent process too. Value must be "true":
    // the pager's clap bool arg (`env = "GROK_LOG_SAMPLING"`) rejects "1",
    // while the sampling layer accepts "1" | "true" | "on".
    let mut env = content.env_for_pager();
    env.push(("GROK_LOG_SAMPLING".into(), "true".into()));
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut harness =
        PtyHarness::new(&binary, DEFAULT_ROWS, DEFAULT_COLS, &[], &env_refs).expect("spawn pager");

    harness
        .wait_for_text(WELCOME_SCREEN_SENTINEL, WELCOME_TIMEOUT)
        .expect("welcome text");

    harness
        .inject_keys(format!("{PROMPT}\r").as_bytes())
        .expect("submit prompt");
    harness
        .wait_for_text(MOCK_RESPONSE_SENTINEL, Duration::from_secs(30))
        .expect("response rendered");

    // Quit via Ctrl+Q double-press (focus is in the prompt, so 'q' would type).
    harness.update(Duration::from_millis(500));
    harness.inject_keys(b"\x11").expect("ctrl-q once");
    harness.update(Duration::from_millis(200));
    harness.inject_keys(b"\x11").expect("ctrl-q confirm");
    harness.quit().expect("reap pager");

    let unified = read_matching_log(&grok_home, "logs/unified");
    assert!(
        unified.contains("pager started"),
        "unified.jsonl must carry the pager startup entry\ncontents:\n{unified}"
    );

    let sampling = read_matching_log(&grok_home, "logs/sampling");
    assert!(
        sampling.contains("Received prompt request"),
        "sampling.jsonl must carry the agent prompt event\ncontents:\n{sampling}"
    );

    // ── Run 2: debug firehose on (sampling off) → debug/*.txt ──────────────
    let mut env = content.env_for_pager();
    env.push(("GROK_DEBUG_LOG".into(), "1".into()));
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut harness = PtyHarness::new(&binary, DEFAULT_ROWS, DEFAULT_COLS, &[], &env_refs)
        .expect("spawn pager for debug run");
    harness
        .wait_for_text(WELCOME_SCREEN_SENTINEL, WELCOME_TIMEOUT)
        .expect("welcome text (debug run)");
    harness.update(Duration::from_millis(500));
    harness
        .inject_keys(b"\x11")
        .expect("ctrl-q once (debug run)");
    harness.update(Duration::from_millis(200));
    harness
        .inject_keys(b"\x11")
        .expect("ctrl-q confirm (debug run)");
    harness.quit().expect("reap debug-run pager");

    let debug = read_matching_log(&grok_home, "debug/");
    assert!(
        !debug.trim().is_empty(),
        "debug firehose files must not be empty"
    );
}

/// Concatenate every file under `grok_home` whose grok-home-relative path
/// contains `needle` (e.g. `"logs/unified"` matches `logs/unified.jsonl`,
/// `"debug/"` matches every per-session firehose file). Panics with a full
/// recursive directory listing when nothing matches, so a relocated log dir
/// fails loud instead of vacuously.
fn read_matching_log(grok_home: &Path, needle: &str) -> String {
    let mut files = Vec::new();
    collect_files(grok_home, &mut files);

    let mut combined = String::new();
    let mut matched = 0usize;
    for file in &files {
        let rel = file
            .strip_prefix(grok_home)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains(needle) {
            matched += 1;
            combined.push_str(&std::fs::read_to_string(file).unwrap_or_default());
        }
    }

    assert!(
        matched > 0,
        "no log file matching {needle:?} under {}\nfiles present:\n{}",
        grok_home.display(),
        files
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    combined
}

fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Cheap, non-ignored CI guard: the pager's tracing bootstrap must keep
/// installing every retained local diagnostic layer. Complements the ignored
/// PTY case above so ordinary `cargo test` runs (which skip PTY families)
/// still cover the wiring.
#[test]
fn pager_tracing_source_installs_local_diagnostic_layers() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tracing.rs"));
    for required in [
        "instrumentation::layer()",
        "sampling_log::layer()",
        "hooks_log::layer()",
        "debug_log::install_firehose",
    ] {
        assert!(
            source.contains(required),
            "local tracing layer missing from pager tracing bootstrap: {required}"
        );
    }
}
