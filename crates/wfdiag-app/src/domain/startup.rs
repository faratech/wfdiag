//! The startup-scan gate.
//!
//! A scan requested by the `scanOnStartup` setting may only run once, and only
//! after the settings load and the system-identity probes finish: the admin
//! flag decides which tasks are eligible. The gate is consumed *before*
//! dispatch so a rejected startup scan is never retried.

/// Whether the once-per-launch startup scan may still run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartupScanGate {
    /// Settings have not loaded yet, so the preference is unknown.
    #[default]
    AwaitingSettings,
    /// The preference is on and the scan has not run.
    Armed,
    /// The gate has been used (or the preference is off).
    Consumed,
}

impl StartupScanGate {
    /// Apply the loaded `scanOnStartup` preference exactly once.
    pub fn apply_preference(&mut self, scan_on_startup: bool) {
        if *self == Self::AwaitingSettings {
            *self = if scan_on_startup {
                Self::Armed
            } else {
                Self::Consumed
            };
        }
    }

    /// Permanently close the gate (a host that forbids startup scans).
    pub fn consume(&mut self) {
        *self = Self::Consumed;
    }

    /// Take the startup scan when every prerequisite has landed.
    ///
    /// The gate is consumed before the caller dispatches, so a runtime that
    /// refuses the scan does not leave a retryable gate behind.
    pub fn take_when_ready(&mut self, readiness: StartupReadiness) -> bool {
        if !readiness.allowed {
            *self = Self::Consumed;
            return false;
        }
        if *self != Self::Armed
            || readiness.settings_loading
            || readiness.system_info_pending
            || readiness.architecture_pending
        {
            return false;
        }
        *self = Self::Consumed;
        true
    }
}

/// The prerequisites a startup scan waits for.
// Four independent prerequisites; collapsing them would hide which one is
// still outstanding, which is exactly what the caller logs.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupReadiness {
    /// Whether the host permits automatic scans at all.
    pub allowed: bool,
    /// A settings load is still in flight.
    pub settings_loading: bool,
    /// The system-information probe has not answered.
    pub system_info_pending: bool,
    /// The architecture probe has not answered.
    pub architecture_pending: bool,
}

#[cfg(test)]
mod tests {
    use super::{StartupReadiness, StartupScanGate};

    fn ready() -> StartupReadiness {
        StartupReadiness {
            allowed: true,
            settings_loading: false,
            system_info_pending: false,
            architecture_pending: false,
        }
    }

    #[test]
    fn the_preference_arms_the_gate_exactly_once() {
        let mut gate = StartupScanGate::default();
        gate.apply_preference(true);
        assert_eq!(gate, StartupScanGate::Armed);
        gate.apply_preference(false);
        assert_eq!(gate, StartupScanGate::Armed, "a reload cannot re-decide");
    }

    #[test]
    fn a_disabled_preference_consumes_the_gate() {
        let mut gate = StartupScanGate::default();
        gate.apply_preference(false);
        assert_eq!(gate, StartupScanGate::Consumed);
        assert!(!gate.take_when_ready(ready()));
    }

    #[test]
    fn the_scan_waits_for_settings_and_identity_then_runs_once() {
        let mut gate = StartupScanGate::default();
        gate.apply_preference(true);
        assert!(!gate.take_when_ready(StartupReadiness {
            settings_loading: true,
            ..ready()
        }));
        assert!(!gate.take_when_ready(StartupReadiness {
            system_info_pending: true,
            ..ready()
        }));
        assert!(!gate.take_when_ready(StartupReadiness {
            architecture_pending: true,
            ..ready()
        }));
        assert!(gate.take_when_ready(ready()));
        assert!(!gate.take_when_ready(ready()), "the gate is single-use");
    }

    #[test]
    fn a_host_that_forbids_startup_scans_closes_the_gate() {
        let mut gate = StartupScanGate::default();
        gate.apply_preference(true);
        assert!(!gate.take_when_ready(StartupReadiness {
            allowed: false,
            ..ready()
        }));
        assert_eq!(gate, StartupScanGate::Consumed);
    }
}
