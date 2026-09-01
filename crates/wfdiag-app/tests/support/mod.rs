//! Shared harness for the headless integration tests.
//!
//! Every test drives the real [`AppService`] — real worker threads, real
//! channels, real staleness guards — with the in-memory ports from
//! `wfdiag_app::ports::mock`. Nothing here touches the network, the registry,
//! `WinRT`, or a GUI, so the whole suite runs on Linux.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wfdiag_app::ports::mock::MockPorts;
use wfdiag_app::{AppCommand, AppConfig, AppEvent, AppEventReceiver, AppService};

/// A temporary directory removed when the harness drops.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wfdiag_app_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the temporary history directory");
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A started service plus everything a test needs to script it.
pub struct Harness {
    pub service: AppService,
    pub events: AppEventReceiver,
    pub mocks: MockPorts,
    pub directory: TempDir,
    /// Everything drained while bringing the service to its first frame.
    pub startup_events: Vec<AppEvent>,
}

/// The default test configuration: real history in a temporary directory,
/// live monitoring on, release builds (so update checks are not silenced),
/// and a short reply deadline so a stalled worker fails fast.
pub fn test_config(directory: &TempDir) -> AppConfig {
    AppConfig::default()
        .with_history_dir(directory.path())
        .with_monitor(true)
        .with_debug_build(false)
        .with_reply_timeout(Duration::from_secs(5))
}

/// Start a service with the given mocks and configuration, without dispatching
/// [`AppCommand::Start`].
pub fn start_with(
    label: &str,
    mocks: MockPorts,
    config: impl FnOnce(AppConfig) -> AppConfig,
) -> Harness {
    let directory = TempDir::new(label);
    let config = config(test_config(&directory));
    let (service, events) =
        AppService::start(config, mocks.to_ports()).expect("the service starts headlessly");
    Harness {
        service,
        events,
        mocks,
        directory,
        startup_events: Vec::new(),
    }
}

/// Start a service and bring it to the state a shell reaches after its first
/// frame: started, settings loaded, host identity known.
pub fn boot(label: &str) -> Harness {
    boot_with(label, MockPorts::new())
}

/// As [`boot`], with caller-supplied mocks.
pub fn boot_with(label: &str, mocks: MockPorts) -> Harness {
    let mut harness = start_with(label, mocks, |config| config);
    assert!(
        harness
            .service
            .dispatch(AppCommand::Start {
                startup_scan: false
            })
            .is_accepted()
    );
    let startup_events = harness.pump_until(
        |harness, _| {
            !harness.service.snapshot().settings_loading
                && harness.service.snapshot().system_info.is_some()
        },
        "settings and host identity",
    );
    harness.startup_events = startup_events;
    harness
}

impl Harness {
    /// Drain repeatedly until `ready` holds, collecting every event.
    ///
    /// Panics with the collected events if `ready` has not held within two
    /// seconds, so a broken guard fails the test instead of hanging it.
    pub fn pump_until(
        &mut self,
        ready: impl Fn(&Self, &[AppEvent]) -> bool,
        what: &str,
    ) -> Vec<AppEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut collected = Vec::new();
        loop {
            collected.extend(self.service.drain());
            if ready(self, &collected) {
                return collected;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; collected {} events: {collected:#?}",
                collected.len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Drain until at least one event matches `matches`.
    pub fn pump_for(&mut self, what: &str, matches: impl Fn(&AppEvent) -> bool) -> Vec<AppEvent> {
        self.pump_until(|_, events| events.iter().any(&matches), what)
    }

    /// Drain everything currently available, with a short settle window.
    pub fn pump_briefly(&mut self) -> Vec<AppEvent> {
        let mut collected = Vec::new();
        for _ in 0..10 {
            collected.extend(self.service.drain());
            std::thread::sleep(Duration::from_millis(5));
        }
        collected.extend(self.service.drain());
        collected
    }

    /// Consume the harness, shutting the service down inside `budget`.
    pub fn shutdown(self, budget: Duration) -> wfdiag_app::ShutdownReport {
        let Self {
            mut service,
            events,
            mocks,
            directory,
            startup_events: _,
        } = self;
        let _ = service.dispatch(AppCommand::Shutdown);
        let report = service.shutdown(budget);
        assert!(events.is_terminated(), "the receiver observes termination");
        drop(mocks);
        drop(directory);
        report
    }
}

/// Mocks whose provider probes make Ollama (local) and `OpenAI` (cloud)
/// available, which is the exact shape an `Auto` local-to-cloud fallback
/// needs.
#[must_use]
pub fn ai_mocks() -> MockPorts {
    let mocks = MockPorts::new();
    mocks
        .provider_backend
        .set_probes(wfdiag_native_ai_provider::ProviderProbeSnapshot {
            ollama_endpoint: Some("http://127.0.0.1:11434".to_string()),
            openai_available: true,
            ..wfdiag_native_ai_provider::ProviderProbeSnapshot::default()
        });
    mocks
}

/// Boot with [`ai_mocks`] and refresh the provider status, so every AI
/// command's readiness gate is satisfied before the test starts.
pub fn boot_ai(label: &str) -> Harness {
    boot_ai_with(label, ai_mocks())
}

/// As [`boot_ai`], with caller-supplied mocks.
pub fn boot_ai_with(label: &str, mocks: MockPorts) -> Harness {
    let mut harness = boot_with(label, mocks);
    assert!(
        harness
            .service
            .dispatch(AppCommand::RequestProviderStatus)
            .is_accepted()
    );
    harness.pump_until(
        |harness, _| harness.service.snapshot().provider_status.is_some(),
        "the AI provider status",
    );
    harness
}

impl Harness {
    /// Run a quick scan and wait for the committed issue projection, which is
    /// the state every evidence-dependent AI command needs.
    pub fn commit_scan(&mut self) {
        assert!(
            self.service
                .dispatch(AppCommand::StartScan {
                    kind: wfdiag_native_diagnostics::ScanKind::Quick,
                })
                .is_accepted()
        );
        self.pump_until(
            |harness, events| {
                !harness.service.snapshot().scan.results.is_empty()
                    && !harness.service.snapshot().scan_busy()
                    && events.iter().any(|event| {
                        matches!(
                            event,
                            AppEvent::Issues(wfdiag_app::IssuesEvent::Updated { .. })
                        )
                    })
            },
            "the scan to commit, finalize, and project issues",
        );
    }
}
