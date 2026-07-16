//! Threshold-triggered jemalloc heap dump, persisted locally.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Hard skip-cap for dumps (K4). Allowed sizes are `1..=HARD_DUMP_SIZE_CAP_BYTES`.
pub const HARD_DUMP_SIZE_CAP_BYTES: u64 = 128 * 1024 * 1024;

/// Wall budget for `prof.dump` in `spawn_blocking` (K6).
pub const DUMP_TIMEOUT: Duration = Duration::from_secs(30);

/// Scoped kill-switch poll cadence while profiling is enabled (K12).
pub const SCOPED_KILL_SWITCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

const DEFAULT_POLL_INTERVAL_SECS: u64 = 30;
const MIN_POLL_INTERVAL_SECS: u64 = 5;
const MAX_POLL_INTERVAL_SECS: u64 = 300;

/// Resolved jemalloc heap-profile runtime config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JemallocHeapProfileConfig {
    pub enabled: bool,
    /// Sorted unique ascending thresholds (bytes of `stats.resident`).
    pub thresholds: Vec<u64>,
    pub poll_interval: Duration,
}

impl Default for JemallocHeapProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            thresholds: Vec::new(),
            poll_interval: Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS),
        }
    }
}

/// Outcome of one threshold dump attempt (for latch decisions / tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpAttemptOutcome {
    /// Missing session — do not latch (K6 / defer).
    Deferred,
    DumpFailed,
    DumpTimeout,
    SizeCap,
    PersistOk,
    PersistFailed,
}

/// Whether a dump-attempt outcome should latch the threshold (K6).
pub fn should_latch(outcome: DumpAttemptOutcome) -> bool {
    !matches!(outcome, DumpAttemptOutcome::Deferred)
}

/// True when `session_id` is a UUID (path_auth Session class leading segment).
pub fn is_valid_session_id(session_id: &str) -> bool {
    uuid::Uuid::try_parse(session_id).is_ok()
}

/// Sanitize a binary version for object leaf names (`[A-Za-z0-9._-]`, collapse `_`).
pub fn sanitize_version(version: &str) -> String {
    let mut out = String::with_capacity(version.len());
    let mut prev_us = false;
    for c in version.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        if ok {
            out.push(c);
            prev_us = c == '_';
        } else if !prev_us {
            out.push('_');
            prev_us = true;
        }
    }
    let trim_end = out.trim_end_matches('_').len();
    out.truncate(trim_end);
    let trim_start = out.len() - out.trim_start_matches('_').len();
    if trim_start > 0 {
        out.drain(..trim_start);
    }
    if out.is_empty() {
        out.push_str("unknown");
    }
    out
}

/// `{session_id}/jemalloc/{session_id}-{version}-{ts}.heap` (+ `.meta.json`).
pub fn object_paths(session_id: &str, version: &str, ts_unix: u64) -> (String, String) {
    let ver = sanitize_version(version);
    let base = format!("{session_id}/jemalloc/{session_id}-{ver}-{ts_unix}");
    (format!("{base}.heap"), format!("{base}.meta.json"))
}

pub fn normalize_thresholds(thresholds: impl IntoIterator<Item = u64>) -> Vec<u64> {
    let mut t: Vec<u64> = thresholds.into_iter().collect();
    t.sort_unstable();
    t.dedup();
    t
}

/// Clamp poll interval seconds to `5..=300`, default 30 when absent.
pub fn clamp_poll_interval_secs(secs: Option<u64>) -> u64 {
    secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
        .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS)
}

pub fn resolve_jemalloc_heap_profile(
    remote_enabled: Option<bool>,
    remote_thresholds: Option<&[u64]>,
    remote_poll_interval_secs: Option<u64>,
    data_collection_disabled: bool,
    prof_available: bool,
) -> JemallocHeapProfileConfig {
    let thresholds = match remote_thresholds {
        Some(t) if !t.is_empty() => normalize_thresholds(t.iter().copied()),
        _ => Vec::new(),
    };
    let enabled = remote_enabled == Some(true)
        && !thresholds.is_empty()
        && !data_collection_disabled
        && prof_available;
    JemallocHeapProfileConfig {
        enabled,
        thresholds,
        poll_interval: Duration::from_secs(clamp_poll_interval_secs(remote_poll_interval_secs)),
    }
}

/// Process-lifetime latch + dump/persist orchestration for heap profiles.
pub struct HeapProfileMonitor {
    latched: BTreeSet<u64>,
    config: JemallocHeapProfileConfig,
    /// Sticky UUID; set only via [`set_session_id`].
    session_id: Option<Arc<str>>,
    upload_in_flight: bool,
    /// Local directory heap profiles are persisted under.
    persist_dir: PathBuf,
    dump_fn: fn(&Path) -> Result<(), String>,
    stats_fn: fn() -> Option<super::JemallocStats>,
    set_prof_active_fn: fn(bool) -> bool,
    sample_rss_fn: fn() -> u64,
    test_persist: Option<Arc<TestPersistFn>>,
    dump_timeout: Duration,
}

type TestPersistFn = dyn Fn(
        &str,
        &Path,
        &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
    + Send
    + Sync;

/// Default local directory for persisted heap profiles.
pub fn default_persist_dir() -> PathBuf {
    crate::util::grok_home::grok_home().join("heap_profiles")
}

impl Default for HeapProfileMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl HeapProfileMonitor {
    pub fn new() -> Self {
        Self {
            latched: BTreeSet::new(),
            config: JemallocHeapProfileConfig::default(),
            session_id: None,
            upload_in_flight: false,
            persist_dir: default_persist_dir(),
            dump_fn: super::dump_to_path,
            stats_fn: super::stats,
            set_prof_active_fn: super::set_prof_active,
            sample_rss_fn: crate::session::signals::sample_rss_bytes,
            test_persist: None,
            dump_timeout: DUMP_TIMEOUT,
        }
    }

    pub fn config(&self) -> &JemallocHeapProfileConfig {
        &self.config
    }

    pub fn latched(&self) -> &BTreeSet<u64> {
        &self.latched
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn upload_in_flight(&self) -> bool {
        self.upload_in_flight
    }

    pub fn clear_upload_in_flight(&mut self) {
        self.upload_in_flight = false;
    }

    /// Apply resolved config; toggle sampling. Does not touch
    /// sticky `session_id` or clear latches.
    pub fn reconfigure(&mut self, config: JemallocHeapProfileConfig) {
        let was_enabled = self.config.enabled;
        self.config = config;
        let active = self.config.enabled;
        let ok = (self.set_prof_active_fn)(active);
        tracing::debug!(
            enabled = active,
            thresholds = ?self.config.thresholds,
            poll_interval_secs = self.config.poll_interval.as_secs(),
            prof_available = super::prof_available(),
            set_prof_active_ok = ok,
            was_enabled,
            "heap_profile: configured"
        );
        if active != was_enabled {
            tracing::info!(active, "heap_profile: prof_active");
        }
    }

    /// Set sticky session id once (first valid UUID wins).
    pub fn set_session_id(&mut self, session_id: String) {
        if self.session_id.is_some() {
            return;
        }
        if !is_valid_session_id(&session_id) {
            tracing::debug!(
                reason = "invalid_session_id",
                "heap_profile: session id rejected (need UUID for path_auth)"
            );
            return;
        }
        self.session_id = Some(Arc::from(session_id));
    }

    /// Start a dump when a threshold is crossed. Deferred paths return `None`
    /// without latching.
    pub fn begin_tick(&mut self) -> Option<PendingDump> {
        if !self.config.enabled || self.upload_in_flight {
            if self.upload_in_flight {
                tracing::debug!(reason = "in_flight", "heap_profile: skipped");
            }
            return None;
        }
        let stats = (self.stats_fn)()?;
        let threshold = self
            .config
            .thresholds
            .iter()
            .copied()
            .find(|t| stats.resident >= *t && !self.latched.contains(t))?;

        let Some(session_id) = self.session_id.clone() else {
            tracing::debug!(threshold, reason = "no_session", "heap_profile: skipped");
            return None;
        };

        self.upload_in_flight = true;
        Some(PendingDump {
            threshold,
            stats,
            session_id,
            rss_peak: (self.sample_rss_fn)(),
            dump_fn: self.dump_fn,
            dump_timeout: self.dump_timeout,
            persist_dir: self.persist_dir.clone(),
            test_persist: self.test_persist.clone(),
        })
    }

    pub fn finish_tick(&mut self, threshold: u64, outcome: DumpAttemptOutcome) {
        if should_latch(outcome) {
            self.latched.insert(threshold);
        }
        self.upload_in_flight = false;
    }

    pub async fn poll_tick(&mut self) {
        let Some(pending) = self.begin_tick() else {
            return;
        };
        let threshold = pending.threshold;
        let outcome = pending.execute().await;
        self.finish_tick(threshold, outcome);
    }

    #[cfg(test)]
    pub(crate) fn with_test_hooks(
        mut self,
        dump_fn: fn(&Path) -> Result<(), String>,
        stats_fn: fn() -> Option<super::JemallocStats>,
        set_prof_active_fn: fn(bool) -> bool,
        sample_rss_fn: fn() -> u64,
    ) -> Self {
        self.dump_fn = dump_fn;
        self.stats_fn = stats_fn;
        self.set_prof_active_fn = set_prof_active_fn;
        self.sample_rss_fn = sample_rss_fn;
        self
    }

    #[cfg(test)]
    pub(crate) fn set_dump_timeout(&mut self, timeout: Duration) {
        self.dump_timeout = timeout;
    }

    /// Override the local persist directory (tests).
    pub fn set_persist_dir(&mut self, dir: PathBuf) {
        self.persist_dir = dir;
    }

    #[cfg(test)]
    pub(crate) fn set_test_persist<F>(&mut self, f: F)
    where
        F: Fn(
                &str,
                &Path,
                &str,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
            + Send
            + Sync
            + 'static,
    {
        self.test_persist = Some(Arc::new(f));
    }

    #[cfg(test)]
    pub(crate) fn force_latched(&mut self, thresholds: impl IntoIterator<Item = u64>) {
        self.latched = thresholds.into_iter().collect();
    }
}

/// Work item produced by [`HeapProfileMonitor::begin_tick`].
pub struct PendingDump {
    pub threshold: u64,
    stats: super::JemallocStats,
    session_id: Arc<str>,
    rss_peak: u64,
    dump_fn: fn(&Path) -> Result<(), String>,
    dump_timeout: Duration,
    persist_dir: PathBuf,
    test_persist: Option<Arc<TestPersistFn>>,
}

impl PendingDump {
    /// Dump + local persist off the monitor borrow. On timeout, awaits the dump
    /// join before returning so in-flight stays set until the private dir is safe.
    pub async fn execute(self) -> DumpAttemptOutcome {
        let threshold = self.threshold;
        let stats = self.stats;
        let session_id = self.session_id.as_ref();
        let rss_peak = self.rss_peak;
        let ts_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let version = xai_grok_version::installed();

        tracing::warn!(
            threshold,
            resident = stats.resident,
            allocated = stats.allocated,
            rss_peak_bytes = rss_peak,
            session_id,
            "heap_profile: threshold_crossed"
        );

        let temp_dir = match PrivateTempDir::create() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "heap_profile: dump_failed");
                return DumpAttemptOutcome::DumpFailed;
            }
        };
        let temp_path = temp_dir.path().join("dump.heap");

        let dump_fn = self.dump_fn;
        let dump_path = temp_path.clone();
        let dump_start = std::time::Instant::now();
        let mut handle = tokio::task::spawn_blocking(move || dump_fn(&dump_path));

        let dump_result: Result<(), DumpAttemptOutcome> =
            match tokio::time::timeout(self.dump_timeout, &mut handle).await {
                Ok(Ok(Ok(()))) => Ok(()),
                Ok(Ok(Err(e))) => {
                    tracing::warn!(
                        path = %temp_path.display(),
                        elapsed_ms = dump_start.elapsed().as_millis() as u64,
                        error = %e,
                        "heap_profile: dump_failed"
                    );
                    Err(DumpAttemptOutcome::DumpFailed)
                }
                Ok(Err(join_err)) => {
                    tracing::warn!(
                        path = %temp_path.display(),
                        elapsed_ms = dump_start.elapsed().as_millis() as u64,
                        error = %join_err,
                        "heap_profile: dump_failed"
                    );
                    Err(DumpAttemptOutcome::DumpFailed)
                }
                Err(_) => {
                    tracing::warn!(
                        path = %temp_path.display(),
                        elapsed_ms = dump_start.elapsed().as_millis() as u64,
                        "heap_profile: dump_timeout"
                    );
                    let _ = handle.await;
                    Err(DumpAttemptOutcome::DumpTimeout)
                }
            };

        if let Err(outcome) = dump_result {
            return outcome;
        }

        tracing::info!(
            path = %temp_path.display(),
            elapsed_ms = dump_start.elapsed().as_millis() as u64,
            "heap_profile: dump_ok"
        );

        let file_size = match std::fs::metadata(&temp_path) {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::warn!(
                    path = %temp_path.display(),
                    error = %e,
                    "heap_profile: dump_failed"
                );
                return DumpAttemptOutcome::DumpFailed;
            }
        };

        if file_size == 0 || file_size > HARD_DUMP_SIZE_CAP_BYTES {
            tracing::debug!(
                reason = "size_cap",
                bytes = file_size,
                "heap_profile: skipped"
            );
            return DumpAttemptOutcome::SizeCap;
        }

        let (heap_object, meta_object) = object_paths(session_id, &version, ts_unix);
        let meta = serde_json::json!({
            "session_id": session_id,
            "binary_version": version,
            "threshold_bytes": threshold,
            "stats_resident": stats.resident,
            "stats_allocated": stats.allocated,
            "rss_peak_bytes": rss_peak,
            "ts_unix": ts_unix,
            "os": std::env::consts::OS,
            "lg_prof_sample": crate::heap_profile::LG_PROF_SAMPLE,
        });
        let meta_bytes = match serde_json::to_vec(&meta) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "heap_profile: dump_failed");
                return DumpAttemptOutcome::DumpFailed;
            }
        };

        let meta_path = temp_dir.path().join("dump.meta.json");
        if let Err(e) = write_exclusive_private(&meta_path, &meta_bytes) {
            tracing::warn!(
                path = %meta_path.display(),
                error = %e,
                "heap_profile: dump_failed"
            );
            return DumpAttemptOutcome::DumpFailed;
        }

        let persist_ok = persist_pair(
            self.test_persist.as_deref(),
            &self.persist_dir,
            &heap_object,
            &temp_path,
            "application/octet-stream",
            &meta_object,
            &meta_path,
            "application/json",
            file_size,
        )
        .await;

        if persist_ok {
            DumpAttemptOutcome::PersistOk
        } else {
            DumpAttemptOutcome::PersistFailed
        }
    }
}

fn log_persist_result(heap_object: &str, file_size: u64, ok: bool, err: Option<&str>) -> bool {
    if ok {
        tracing::info!(
            object_path = %heap_object,
            bytes = file_size,
            "heap_profile: persist_ok"
        );
        true
    } else {
        tracing::warn!(
            object_path = %heap_object,
            bytes = file_size,
            error = err.unwrap_or("unknown"),
            "heap_profile: persist_failed"
        );
        false
    }
}

/// Copy `src` to `persist_dir/relative` (creating parents).
fn persist_file(persist_dir: &Path, relative: &str, src: &Path) -> Result<(), String> {
    let dest = persist_dir.join(relative);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Copy (not rename): the source lives in a private temp dir that may be
    // on a different filesystem.
    std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

async fn persist_pair(
    test_persist: Option<&TestPersistFn>,
    persist_dir: &Path,
    heap_object: &str,
    heap_path: &Path,
    heap_ct: &str,
    meta_object: &str,
    meta_path: &Path,
    meta_ct: &str,
    file_size: u64,
) -> bool {
    // Short-circuit on heap failure: do not persist an orphan `.meta.json`.
    if let Some(hook) = test_persist {
        if let Err(e) = hook(heap_object, heap_path, heap_ct).await {
            return log_persist_result(heap_object, file_size, false, Some(&e));
        }
        return match hook(meta_object, meta_path, meta_ct).await {
            Ok(()) => log_persist_result(heap_object, file_size, true, None),
            Err(e) => log_persist_result(heap_object, file_size, false, Some(&e)),
        };
    }

    if let Err(e) = persist_file(persist_dir, heap_object, heap_path) {
        return log_persist_result(heap_object, file_size, false, Some(&e));
    }
    match persist_file(persist_dir, meta_object, meta_path) {
        Ok(()) => log_persist_result(heap_object, file_size, true, None),
        Err(e) => log_persist_result(heap_object, file_size, false, Some(&e)),
    }
}

struct PrivateTempDir {
    path: PathBuf,
}

impl PrivateTempDir {
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "grok-jemalloc-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_exclusive_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    static TEST_RESIDENT: AtomicU64 = AtomicU64::new(0);
    static TEST_ALLOCATED: AtomicU64 = AtomicU64::new(0);
    static TEST_DUMP_FAIL: AtomicBool = AtomicBool::new(false);
    static TEST_PROF_ACTIVE: AtomicBool = AtomicBool::new(false);
    static TEST_STATS_NONE: AtomicBool = AtomicBool::new(false);
    static TEST_DUMP_BYTES: AtomicU64 = AtomicU64::new(1024);
    static TEST_DUMP_SLEEP_MS: AtomicU64 = AtomicU64::new(0);
    static LAST_DUMP_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

    const SID: &str = "11111111-1111-4111-8111-111111111111";

    fn test_stats() -> Option<super::super::JemallocStats> {
        if TEST_STATS_NONE.load(Ordering::SeqCst) {
            return None;
        }
        Some(super::super::JemallocStats {
            allocated: TEST_ALLOCATED.load(Ordering::SeqCst),
            resident: TEST_RESIDENT.load(Ordering::SeqCst),
        })
    }

    fn test_dump(path: &Path) -> Result<(), String> {
        *LAST_DUMP_PATH.lock().unwrap() = Some(path.to_path_buf());
        let sleep_ms = TEST_DUMP_SLEEP_MS.load(Ordering::SeqCst);
        if sleep_ms > 0 {
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
        if TEST_DUMP_FAIL.load(Ordering::SeqCst) {
            return Err("test dump failed".into());
        }
        let n = TEST_DUMP_BYTES.load(Ordering::SeqCst);
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        f.set_len(n).map_err(|e| e.to_string())
    }

    fn test_set_active(active: bool) -> bool {
        TEST_PROF_ACTIVE.store(active, Ordering::SeqCst);
        true
    }

    fn test_rss() -> u64 {
        7_100_000_000
    }

    fn reset() {
        TEST_RESIDENT.store(0, Ordering::SeqCst);
        TEST_ALLOCATED.store(1_000, Ordering::SeqCst);
        TEST_DUMP_FAIL.store(false, Ordering::SeqCst);
        TEST_PROF_ACTIVE.store(false, Ordering::SeqCst);
        TEST_STATS_NONE.store(false, Ordering::SeqCst);
        TEST_DUMP_BYTES.store(1024, Ordering::SeqCst);
        TEST_DUMP_SLEEP_MS.store(0, Ordering::SeqCst);
        *LAST_DUMP_PATH.lock().unwrap() = None;
    }

    fn enabled_config(thresholds: &[u64]) -> JemallocHeapProfileConfig {
        JemallocHeapProfileConfig {
            enabled: true,
            thresholds: normalize_thresholds(thresholds.iter().copied()),
            poll_interval: Duration::from_secs(30),
        }
    }

    fn monitor() -> HeapProfileMonitor {
        HeapProfileMonitor::new().with_test_hooks(test_dump, test_stats, test_set_active, test_rss)
    }

    fn ready_monitor(thresholds: &[u64]) -> HeapProfileMonitor {
        let mut mon = monitor();
        mon.reconfigure(enabled_config(thresholds));
        mon.set_session_id(SID.to_owned());
        mon.set_test_persist(|_, _, _| Box::pin(async { Ok(()) }));
        mon
    }

    #[test]
    fn sanitize_version_replaces_and_collapses() {
        assert_eq!(sanitize_version("0.2.5"), "0.2.5");
        assert_eq!(sanitize_version("0.2.5 (abc1234)"), "0.2.5_abc1234");
        assert_eq!(sanitize_version("a//b"), "a_b");
        assert_eq!(sanitize_version("___"), "unknown");
        assert_eq!(sanitize_version(""), "unknown");
        assert_eq!(sanitize_version("v1.0-rc.1"), "v1.0-rc.1");
    }

    #[test]
    fn is_valid_session_id_uuid_only() {
        assert!(is_valid_session_id(SID));
        assert!(is_valid_session_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_session_id("sess-1"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("not-a-uuid"));
    }

    #[test]
    fn object_paths_session_scoped() {
        let (heap, meta) = object_paths(SID, "0.2.5 (x)", 1710000000);
        assert_eq!(
            heap,
            format!("{SID}/jemalloc/{SID}-0.2.5_x-1710000000.heap")
        );
        assert_eq!(
            meta,
            format!("{SID}/jemalloc/{SID}-0.2.5_x-1710000000.meta.json")
        );
        assert!(heap.starts_with(&format!("{SID}/")));
        assert!(!heap.starts_with("jemalloc/"));
        assert!(is_valid_session_id(heap.split('/').next().unwrap()));
    }

    #[test]
    fn normalize_thresholds_sorts_and_dedups() {
        assert_eq!(
            normalize_thresholds([5_000, 2_000, 5_000, 10_000]),
            vec![2_000, 5_000, 10_000]
        );
    }

    #[test]
    fn clamp_poll_interval_bounds() {
        assert_eq!(clamp_poll_interval_secs(None), 30);
        assert_eq!(clamp_poll_interval_secs(Some(1)), 5);
        assert_eq!(clamp_poll_interval_secs(Some(60)), 60);
        assert_eq!(clamp_poll_interval_secs(Some(9999)), 300);
    }

    #[test]
    fn resolve_gates_require_all_conditions() {
        let thresholds = [2u64 * 1024 * 1024 * 1024];
        let c = resolve_jemalloc_heap_profile(Some(true), Some(&thresholds), Some(30), false, true);
        assert!(c.enabled);
        assert_eq!(c.thresholds, thresholds);

        assert!(
            !resolve_jemalloc_heap_profile(Some(false), Some(&thresholds), None, false, true,)
                .enabled
        );
        assert!(!resolve_jemalloc_heap_profile(None, Some(&thresholds), None, false, true).enabled);
        assert!(!resolve_jemalloc_heap_profile(Some(true), Some(&[]), None, false, true).enabled);
        assert!(!resolve_jemalloc_heap_profile(Some(true), None, None, false, true).enabled);
        assert!(
            !resolve_jemalloc_heap_profile(Some(true), Some(&thresholds), None, true, true).enabled
        );
        assert!(
            !resolve_jemalloc_heap_profile(Some(true), Some(&thresholds), None, false, false,)
                .enabled
        );
    }

    #[test]
    fn should_latch_rules() {
        assert!(!should_latch(DumpAttemptOutcome::Deferred));
        assert!(should_latch(DumpAttemptOutcome::DumpFailed));
        assert!(should_latch(DumpAttemptOutcome::DumpTimeout));
        assert!(should_latch(DumpAttemptOutcome::SizeCap));
        assert!(should_latch(DumpAttemptOutcome::PersistOk));
        assert!(should_latch(DumpAttemptOutcome::PersistFailed));
    }

    #[test]
    fn session_id_is_sticky_and_rejects_non_uuid() {
        let mut mon = monitor();
        mon.set_session_id("not-uuid".into());
        assert!(mon.session_id().is_none());
        mon.set_session_id(SID.to_owned());
        assert_eq!(mon.session_id(), Some(SID));
        mon.set_session_id("22222222-2222-4222-8222-222222222222".into());
        assert_eq!(mon.session_id(), Some(SID));
        mon.reconfigure(enabled_config(&[1]));
        assert_eq!(mon.session_id(), Some(SID));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn defer_no_session_does_not_latch() {
        reset();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_test_persist(|_, _, _| Box::pin(async { Ok(()) }));
        assert!(TEST_PROF_ACTIVE.load(Ordering::SeqCst));
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().is_empty());
        assert!(LAST_DUMP_PATH.lock().unwrap().is_none());
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn persists_locally_without_test_hook() {
        reset();
        let dir = tempfile::TempDir::new().expect("persist dir");
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_session_id(SID.to_owned());
        mon.set_persist_dir(dir.path().to_path_buf());
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert!(!mon.upload_in_flight());
        let jemalloc_dir = dir.path().join(SID).join("jemalloc");
        let mut names: Vec<String> = std::fs::read_dir(&jemalloc_dir)
            .expect("jemalloc dir exists")
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "heap + meta persisted: {names:?}");
        assert!(names[0].ends_with(".heap"));
        assert!(names[1].ends_with(".meta.json"));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn latch_on_dump_failure() {
        reset();
        TEST_DUMP_FAIL.store(true, Ordering::SeqCst);
        let mut mon = ready_monitor(&[100]);
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        TEST_DUMP_FAIL.store(false, Ordering::SeqCst);
        mon.poll_tick().await;
        assert_eq!(mon.latched().len(), 1);
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn latch_on_size_cap_over() {
        reset();
        TEST_DUMP_BYTES.store(HARD_DUMP_SIZE_CAP_BYTES + 1, Ordering::SeqCst);
        let uploads = Arc::new(AtomicU64::new(0));
        let u = uploads.clone();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_session_id(SID.to_owned());
        mon.set_test_persist(move |_, _, _| {
            u.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert_eq!(uploads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn latch_on_zero_byte_size_cap() {
        reset();
        TEST_DUMP_BYTES.store(0, Ordering::SeqCst);
        let mut mon = ready_monitor(&[100]);
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn exact_hard_cap_is_allowed() {
        reset();
        TEST_DUMP_BYTES.store(HARD_DUMP_SIZE_CAP_BYTES, Ordering::SeqCst);
        let uploads = Arc::new(AtomicU64::new(0));
        let u = uploads.clone();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_session_id(SID.to_owned());
        mon.set_test_persist(move |_, _, _| {
            u.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert_eq!(uploads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn latch_on_upload_failure() {
        reset();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_session_id(SID.to_owned());
        mon.set_test_persist(|_, _, _| Box::pin(async { Err("boom".into()) }));
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn latch_on_upload_success() {
        reset();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_session_id(SID.to_owned());
        let uploads = Arc::new(Mutex::new(Vec::<String>::new()));
        let u = uploads.clone();
        mon.set_test_persist(move |obj, _, _| {
            u.lock().unwrap().push(obj.to_owned());
            Box::pin(async { Ok(()) })
        });
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        let paths = uploads.lock().unwrap().clone();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].starts_with(&format!("{SID}/jemalloc/")));
        assert!(paths[0].ends_with(".heap"));
        assert!(paths[1].ends_with(".meta.json"));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn one_threshold_per_tick() {
        reset();
        let mut mon = ready_monitor(&[100, 200]);
        TEST_RESIDENT.store(500, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert!(!mon.latched().contains(&200));
        mon.poll_tick().await;
        assert!(mon.latched().contains(&200));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn disable_stops_sampling_and_dumps() {
        reset();
        let mut mon = ready_monitor(&[100]);
        assert!(TEST_PROF_ACTIVE.load(Ordering::SeqCst));
        mon.reconfigure(JemallocHeapProfileConfig::default());
        assert!(!TEST_PROF_ACTIVE.load(Ordering::SeqCst));
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().is_empty());
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn re_enable_keeps_prior_latches() {
        reset();
        let mut mon = ready_monitor(&[100, 200]);
        mon.force_latched([100]);
        TEST_RESIDENT.store(500, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert!(mon.latched().contains(&200));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn session_arrives_later_allows_dump() {
        reset();
        let mut mon = monitor();
        mon.reconfigure(enabled_config(&[100]));
        mon.set_test_persist(|_, _, _| Box::pin(async { Ok(()) }));
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().is_empty());
        mon.set_session_id(SID.to_owned());
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn stats_none_and_below_threshold_no_latch() {
        reset();
        let mut mon = ready_monitor(&[1000]);
        TEST_STATS_NONE.store(true, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().is_empty());
        TEST_STATS_NONE.store(false, Ordering::SeqCst);
        TEST_RESIDENT.store(999, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().is_empty());
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn upload_in_flight_blocks_second_begin_tick() {
        reset();
        let mut mon = ready_monitor(&[100, 200]);
        TEST_RESIDENT.store(500, Ordering::SeqCst);
        let pending = mon.begin_tick().expect("first dump");
        assert!(mon.upload_in_flight());
        assert!(mon.begin_tick().is_none());
        let threshold = pending.threshold;
        let outcome = pending.execute().await;
        mon.finish_tick(threshold, outcome);
        assert!(!mon.upload_in_flight());
        assert!(mon.latched().contains(&100));
    }

    #[tokio::test]
    #[serial(heap_profile_monitor)]
    async fn dump_timeout_latches_and_clears_in_flight() {
        reset();
        TEST_DUMP_SLEEP_MS.store(200, Ordering::SeqCst);
        let mut mon = ready_monitor(&[100]);
        mon.set_dump_timeout(Duration::from_millis(20));
        TEST_RESIDENT.store(200, Ordering::SeqCst);
        mon.poll_tick().await;
        assert!(mon.latched().contains(&100));
        assert!(!mon.upload_in_flight());
    }

    #[test]
    fn private_temp_dir_mode_and_cleanup() {
        let dir = PrivateTempDir::create().expect("dir");
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let file = path.join("x.heap");
        write_exclusive_private(&file, b"hi").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(write_exclusive_private(&file, b"again").is_err());
        drop(dir);
        assert!(!path.exists());
    }
}
