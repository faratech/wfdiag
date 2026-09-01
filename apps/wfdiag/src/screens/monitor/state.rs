//! The Monitor screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::app::state::MonitorHistory;
use crate::fixtures::visual::VisualState;
use crate::fixtures::visual::{fixture_monitor_empty_stats, fixture_system_stats};
use wfdiag_app::ports::monitor::NetworkConnection;
use wfdiag_ui_core::SystemStats;

/// Everything the Monitor page renders.
#[derive(Default)]
pub(crate) struct MonitorScreen {
    /// Whether periodic sampling is stopped. Owned here because the Pause /
    /// Resume button is the Monitor page's, but read by the window lifecycle
    /// and by the Processes page, which rides the same tick.
    pub(crate) paused: bool,
    /// A pause the *lifecycle* imposed, which only the lifecycle may lift.
    pub(crate) paused_by_lifecycle: bool,
    pub(crate) stats: Option<SystemStats>,
    pub(crate) history: MonitorHistory,
    pub(crate) error: Option<String>,
    pub(crate) network_connections: Option<Vec<NetworkConnection>>,
    pub(crate) network_loading: bool,
}

impl MonitorScreen {
    /// The first frame, including the deterministic screenshot fixtures.
    pub(crate) fn new(visual_state: VisualState, fixture_mode: bool) -> Self {
        Self {
            stats: match visual_state {
                VisualState::MonitorEmpty => Some(fixture_monitor_empty_stats()),
                VisualState::SettingsBottom => Some(fixture_system_stats()),
                _ if fixture_mode => Some(fixture_system_stats()),
                _ => None,
            },
            history: match visual_state {
                VisualState::MonitorEmpty => {
                    let mut history = MonitorHistory::default();
                    history.push_stats(&fixture_monitor_empty_stats());
                    history
                }
                VisualState::SettingsBottom => MonitorHistory::fixture_258(),
                _ if fixture_mode => MonitorHistory::fixture_258(),
                _ => MonitorHistory::default(),
            },
            ..Self::default()
        }
    }
}

/// Everything the Monitor page can ask for.
#[derive(Clone)]
pub(crate) enum MonitorMsg {
    /// Pause or resume live sampling.
    ToggleMonitoring,
    RequestNetworkConnections,
}
