//! The deterministic health verdict: one score, one band, and the three
//! things to do first — derived only from the issue projection, so it needs
//! no AI provider and explains itself (every penalty names its issue).

use crate::issue_catalog::{Issue, IssueSeverity};
use crate::projection::{IssueCounts, IssueProjection};
use wfdiag_remediation_catalog::{RemediationSummary, RemediationTier};

/// Points a detected Critical issue costs.
pub const CRITICAL_PENALTY: u8 = 25;
/// Points a detected Warning issue costs.
pub const WARNING_PENALTY: u8 = 10;
/// Points a detected Info issue costs.
pub const INFO_PENALTY: u8 = 3;
/// Above this share of undecidable checks the score is only partial.
pub const PARTIAL_CONFIDENCE_UNKNOWN_RATIO: f64 = 0.5;
/// How many "do this first" steps the hero shows.
pub const DO_THIS_FIRST_LIMIT: usize = 3;
/// `NOTIFYICONDATAW.szTip` holds 128 UTF-16 units including the NUL.
pub const TRAY_TOOLTIP_MAX_UTF16: usize = 127;

/// Coarse reading of the score, for colour and copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthBand {
    /// 90 and above.
    Good,
    /// 70 to 89.
    Fair,
    /// 40 to 69.
    NeedsAttention,
    /// Below 40.
    Poor,
}

impl HealthBand {
    #[must_use]
    pub const fn from_score(score: u8) -> Self {
        match score {
            90..=u8::MAX => Self::Good,
            70..=89 => Self::Fair,
            40..=69 => Self::NeedsAttention,
            _ => Self::Poor,
        }
    }

    /// Short label for the hero and the tray.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Fair => "Fair",
            Self::NeedsAttention => "Needs attention",
            Self::Poor => "Poor",
        }
    }
}

/// Whether enough checks were decidable for the score to mean much.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthConfidence {
    Full,
    /// More than half of the checks could not be verified (typically a
    /// standard-user scan skipping the admin-only rules).
    Partial,
}

/// One issue's contribution to the score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthPenalty {
    pub issue_id: String,
    pub points: u8,
}

/// The deterministic verdict for one projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthScore {
    /// 0–100.
    pub score: u8,
    pub band: HealthBand,
    pub penalties: Vec<HealthPenalty>,
    pub verified: usize,
    pub unknown: usize,
    pub confidence: HealthConfidence,
}

impl HealthScore {
    /// One sentence for the hero: "72 · Fair · 2 of 28 checks couldn't be verified".
    #[must_use]
    pub fn summary_text(&self) -> String {
        let base = format!("{} · {}", self.score, self.band.label());
        if self.unknown == 0 {
            return base;
        }
        let total = self.verified + self.unknown;
        format!(
            "{base} · {} of {total} checks couldn't be verified",
            self.unknown
        )
    }
}

/// One "do this first" row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextStep {
    pub issue_id: String,
    pub title: String,
    pub severity: IssueSeverity,
    /// The vetted remediation to offer, when the issue has one.
    pub remediation: Option<RemediationSummary>,
    pub needs_admin: bool,
    /// Why it is on the list ("Critical · one-click fix available").
    pub why: String,
}

const fn severity_penalty(severity: IssueSeverity) -> u8 {
    match severity {
        IssueSeverity::Critical => CRITICAL_PENALTY,
        IssueSeverity::Warning => WARNING_PENALTY,
        IssueSeverity::Info => INFO_PENALTY,
        IssueSeverity::Ok => 0,
    }
}

const fn severity_rank(severity: IssueSeverity) -> u8 {
    match severity {
        IssueSeverity::Critical => 0,
        IssueSeverity::Warning => 1,
        IssueSeverity::Info => 2,
        IssueSeverity::Ok => 3,
    }
}

const fn tier_rank(tier: RemediationTier) -> u8 {
    match tier {
        RemediationTier::OpenTool => 0,
        RemediationTier::AutoSafe => 1,
        RemediationTier::Repair => 2,
    }
}

/// Score the projection. `None` when nothing was checked at all.
///
/// Unknown checks never cost points: a standard-user scan must not look
/// sicker than an elevated one. They lower the confidence instead.
#[must_use]
pub fn health_score(projection: &IssueProjection<'_>) -> Option<HealthScore> {
    let counts = projection.counts;
    if counts.total == 0 {
        return None;
    }
    let penalties: Vec<HealthPenalty> = projection
        .detected
        .iter()
        .map(|issue| HealthPenalty {
            issue_id: issue.id.clone(),
            points: severity_penalty(issue.severity),
        })
        .collect();
    let deducted: u32 = penalties
        .iter()
        .map(|penalty| u32::from(penalty.points))
        .sum();
    let score = u8::try_from(100_u32.saturating_sub(deducted)).unwrap_or(0);
    let verified = counts.total - counts.unknown;
    #[allow(clippy::cast_precision_loss)]
    let unknown_ratio = counts.unknown as f64 / counts.total as f64;
    let confidence = if unknown_ratio > PARTIAL_CONFIDENCE_UNKNOWN_RATIO {
        HealthConfidence::Partial
    } else {
        HealthConfidence::Full
    };
    Some(HealthScore {
        score,
        band: HealthBand::from_score(score),
        penalties,
        verified,
        unknown: counts.unknown,
        confidence,
    })
}

fn next_step(issue: &Issue) -> NextStep {
    let remediation = issue.remediation.clone();
    let needs_admin = remediation
        .as_ref()
        .is_some_and(|remediation| remediation.admin_required);
    let severity = match issue.severity {
        IssueSeverity::Critical => "Critical",
        IssueSeverity::Warning => "Warning",
        IssueSeverity::Info | IssueSeverity::Ok => "Worth a look",
    };
    let action = match remediation.as_ref().map(|remediation| remediation.tier) {
        Some(RemediationTier::OpenTool) => "opens the right Windows tool",
        Some(RemediationTier::AutoSafe) => "one-click safe fix",
        Some(RemediationTier::Repair) => "vetted repair, confirmation required",
        None => "follow the recommendation",
    };
    NextStep {
        issue_id: issue.id.clone(),
        title: issue.title.clone(),
        severity: issue.severity,
        remediation,
        needs_admin,
        why: format!("{severity} · {action}"),
    }
}

/// The detected issues a user should act on first, most valuable first:
/// severity, then whether a vetted remediation exists, then the lightest
/// remediation tier, then catalog order (the projection's order).
#[must_use]
pub fn do_this_first(projection: &IssueProjection<'_>, limit: usize) -> Vec<NextStep> {
    let mut ranked: Vec<(usize, &Issue)> =
        projection.detected.iter().copied().enumerate().collect();
    ranked.sort_by_key(|(position, issue)| {
        (
            severity_rank(issue.severity),
            u8::from(issue.remediation.is_none()),
            issue
                .remediation
                .as_ref()
                .map_or(u8::MAX, |remediation| tier_rank(remediation.tier)),
            *position,
        )
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, issue)| next_step(issue))
        .collect()
}

/// The tray icon's hover text. Always fits `szTip`.
#[must_use]
pub fn health_tray_tooltip(score: Option<&HealthScore>, counts: IssueCounts) -> String {
    let text = match score {
        None => "WindowsForum Diagnostics".to_string(),
        Some(score) if counts.detected == 0 => format!(
            "WindowsForum Diagnostics · Health {} ({})",
            score.score,
            score.band.label()
        ),
        Some(score) => format!(
            "WindowsForum Diagnostics · Health {} ({}) · {} issue{}",
            score.score,
            score.band.label(),
            counts.detected,
            if counts.detected == 1 { "" } else { "s" }
        ),
    };
    truncate_utf16(&text, TRAY_TOOLTIP_MAX_UTF16)
}

fn truncate_utf16(text: &str, max_units: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|character| {
            units += character.len_utf16();
            units <= max_units
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_catalog::IssueStatus;
    use crate::projection::project_issues;

    fn issue(id: &str, severity: IssueSeverity, status: IssueStatus) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity,
            status,
            title: format!("Issue {id}"),
            description: String::new(),
            recommendation: String::new(),
            detected: status == IssueStatus::Detected,
            source_tasks: None,
            remediation: None,
        }
    }

    fn with_remediation(mut issue: Issue, id: &str, tier: RemediationTier, admin: bool) -> Issue {
        issue.remediation = Some(RemediationSummary {
            id: id.to_string(),
            label: id.to_string(),
            description: String::new(),
            tier,
            admin_required: admin,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            batch_eligible: false,
            cancellable: false,
        });
        issue
    }

    #[test]
    fn score_is_none_without_checks() {
        let issues: Vec<Issue> = Vec::new();
        assert_eq!(health_score(&project_issues(&issues)), None);
    }

    #[test]
    fn penalties_are_deterministic_and_floored() {
        let issues = vec![
            issue("a", IssueSeverity::Critical, IssueStatus::Detected),
            issue("b", IssueSeverity::Warning, IssueStatus::Detected),
            issue("c", IssueSeverity::Info, IssueStatus::Detected),
            issue("d", IssueSeverity::Ok, IssueStatus::Ok),
        ];
        let score = health_score(&project_issues(&issues)).unwrap();
        assert_eq!(score.score, 100 - 25 - 10 - 3);
        assert_eq!(score.band, HealthBand::NeedsAttention);
        assert_eq!(
            score
                .penalties
                .iter()
                .map(|penalty| (penalty.issue_id.as_str(), penalty.points))
                .collect::<Vec<_>>(),
            [("a", 25), ("b", 10), ("c", 3)]
        );
        assert_eq!(score.confidence, HealthConfidence::Full);

        let many: Vec<Issue> = (0..6)
            .map(|index| {
                issue(
                    &format!("crit{index}"),
                    IssueSeverity::Critical,
                    IssueStatus::Detected,
                )
            })
            .collect();
        let floored = health_score(&project_issues(&many)).unwrap();
        assert_eq!(floored.score, 0);
        assert_eq!(floored.band, HealthBand::Poor);
        assert_eq!(HealthBand::from_score(100), HealthBand::Good);
        assert_eq!(HealthBand::from_score(70), HealthBand::Fair);
        assert_eq!(HealthBand::from_score(69), HealthBand::NeedsAttention);
    }

    #[test]
    fn unknown_checks_lower_confidence_not_score() {
        let issues = vec![
            issue("a", IssueSeverity::Ok, IssueStatus::Ok),
            issue("b", IssueSeverity::Info, IssueStatus::Unknown),
            issue("c", IssueSeverity::Info, IssueStatus::Unknown),
            issue("d", IssueSeverity::Info, IssueStatus::Skipped),
        ];
        let score = health_score(&project_issues(&issues)).unwrap();
        assert_eq!(score.score, 100);
        assert_eq!(score.confidence, HealthConfidence::Partial);
        assert_eq!(score.verified, 1);
        assert_eq!(score.unknown, 3);
        assert_eq!(
            score.summary_text(),
            "100 · Good · 3 of 4 checks couldn't be verified"
        );
    }

    #[test]
    fn do_this_first_prefers_actionable_open_tools_over_repairs() {
        let issues = vec![
            with_remediation(
                issue("repair", IssueSeverity::Critical, IssueStatus::Detected),
                "sfc_scannow",
                RemediationTier::Repair,
                true,
            ),
            issue("no_fix", IssueSeverity::Critical, IssueStatus::Detected),
            with_remediation(
                issue("tool", IssueSeverity::Critical, IssueStatus::Detected),
                "open_disk_cleanup",
                RemediationTier::OpenTool,
                false,
            ),
            with_remediation(
                issue("warn", IssueSeverity::Warning, IssueStatus::Detected),
                "flush_dns",
                RemediationTier::AutoSafe,
                false,
            ),
            issue("info", IssueSeverity::Info, IssueStatus::Detected),
        ];
        let steps = do_this_first(&project_issues(&issues), DO_THIS_FIRST_LIMIT);
        assert_eq!(
            steps
                .iter()
                .map(|step| step.issue_id.as_str())
                .collect::<Vec<_>>(),
            ["tool", "repair", "no_fix"]
        );
        assert!(steps[1].needs_admin);
        assert_eq!(steps[0].why, "Critical · opens the right Windows tool");
        assert_eq!(steps[2].why, "Critical · follow the recommendation");
        let all = do_this_first(&project_issues(&issues), 10);
        assert_eq!(all.len(), 5);
        assert_eq!(all[3].issue_id, "warn");
        assert_eq!(all[4].why, "Worth a look · follow the recommendation");
    }

    #[test]
    fn tooltip_fits_shell_notify_icon() {
        let issues: Vec<Issue> = (0..40)
            .map(|index| {
                issue(
                    &format!("issue{index}"),
                    IssueSeverity::Warning,
                    IssueStatus::Detected,
                )
            })
            .collect();
        let projection = project_issues(&issues);
        let score = health_score(&projection).unwrap();
        let tooltip = health_tray_tooltip(Some(&score), projection.counts);
        assert!(tooltip.encode_utf16().count() <= TRAY_TOOLTIP_MAX_UTF16);
        assert_eq!(
            tooltip,
            "WindowsForum Diagnostics · Health 0 (Poor) · 40 issues"
        );
        assert_eq!(
            health_tray_tooltip(None, projection.counts),
            "WindowsForum Diagnostics"
        );
        let long = "é".repeat(400);
        assert!(
            truncate_utf16(&long, TRAY_TOOLTIP_MAX_UTF16)
                .encode_utf16()
                .count()
                <= 127
        );
    }
}
