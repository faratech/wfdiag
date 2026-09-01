//! Update-check scheduling.
//!
//! The decision of whether a newer release exists lives in
//! `wfdiag_native_update`. This module owns everything around it: the delay
//! after launch before the passive check runs, the once-a-day throttle, and
//! the rule that a user-initiated check ignores both.

use std::time::Duration;
use wfdiag_native_update::policy;

/// How long after launch the passive check may run.
pub const START_DELAY: Duration = policy::START_DELAY;
/// How long an "update available" notice stays on screen.
pub const NOTICE_DURATION: Duration = policy::NOTICE_DURATION;

/// Why an update check is being requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheckReason {
    /// The passive check that runs shortly after launch.
    Startup,
    /// The user asked, from the About surface.
    Manual,
}

/// What the scheduler decided about one requested check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateSchedule {
    /// Run the check now and record the timestamp.
    Check,
    /// A check already ran inside the throttle window.
    Throttled,
    /// A check is already in flight.
    AlreadyRunning,
}

/// The pure scheduling decision.
///
/// A manual check bypasses the throttle but never runs two checks at once.
#[must_use]
pub const fn schedule(
    reason: UpdateCheckReason,
    in_flight: bool,
    throttle_allows: bool,
) -> UpdateSchedule {
    if in_flight {
        return UpdateSchedule::AlreadyRunning;
    }
    match reason {
        UpdateCheckReason::Manual => UpdateSchedule::Check,
        UpdateCheckReason::Startup => {
            if throttle_allows {
                UpdateSchedule::Check
            } else {
                UpdateSchedule::Throttled
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{START_DELAY, UpdateCheckReason, UpdateSchedule, schedule};
    use std::time::Duration;

    #[test]
    fn the_startup_check_honours_the_throttle_and_a_manual_check_does_not() {
        assert_eq!(
            schedule(UpdateCheckReason::Startup, false, true),
            UpdateSchedule::Check
        );
        assert_eq!(
            schedule(UpdateCheckReason::Startup, false, false),
            UpdateSchedule::Throttled
        );
        assert_eq!(
            schedule(UpdateCheckReason::Manual, false, false),
            UpdateSchedule::Check
        );
    }

    #[test]
    fn no_reason_starts_a_second_concurrent_check() {
        assert_eq!(
            schedule(UpdateCheckReason::Manual, true, true),
            UpdateSchedule::AlreadyRunning
        );
        assert_eq!(
            schedule(UpdateCheckReason::Startup, true, true),
            UpdateSchedule::AlreadyRunning
        );
    }

    #[test]
    fn the_startup_delay_matches_the_shipping_policy() {
        assert_eq!(START_DELAY, Duration::from_secs(5));
    }
}
