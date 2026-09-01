//! What derived content a new scan invalidates, and when.
//!
//! Reports, fix plans, issue prioritisation, and per-task analyses are all
//! derived from committed diagnostic evidence. A replacement scan invalidates
//! them the moment its transaction opens: the evidence they describe is about
//! to disappear. A targeted rerun defers invalidation until its single
//! replacement commits, so every failure path leaves the previous evidence
//! *and* everything derived from it intact.

/// The derived content one transaction invalidates.
// Four independent projections, each genuinely a yes/no. A state machine
// would model one thing changing state; this models four things being dropped.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Invalidation {
    /// The cached AI scan report.
    pub report: bool,
    /// The AI fix plan.
    pub fix_plan: bool,
    /// The AI issue prioritisation.
    pub prioritization: bool,
    /// Per-task AI analyses.
    pub analyses: bool,
}

impl Invalidation {
    /// Nothing is invalidated.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            report: false,
            fix_plan: false,
            prioritization: false,
            analyses: false,
        }
    }

    /// Everything derived from the committed evidence is invalidated.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            report: true,
            fix_plan: true,
            prioritization: true,
            analyses: true,
        }
    }

    /// True when at least one projection must be dropped.
    #[must_use]
    pub const fn any(self) -> bool {
        self.report || self.fix_plan || self.prioritization || self.analyses
    }

    /// The invalidation for a scan transaction that is opening.
    ///
    /// `targeted_rerun` defers to [`Self::on_targeted_commit`].
    #[must_use]
    pub const fn on_scan_start(targeted_rerun: bool) -> Self {
        if targeted_rerun {
            Self::none()
        } else {
            Self::all()
        }
    }

    /// The invalidation applied when a targeted rerun finally commits.
    #[must_use]
    pub const fn on_targeted_commit() -> Self {
        Self::all()
    }

    /// The invalidation applied when new issues are projected. The report and
    /// analyses describe diagnostic output, not issues, so they survive.
    #[must_use]
    pub const fn on_issue_projection() -> Self {
        Self {
            report: false,
            fix_plan: true,
            prioritization: true,
            analyses: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Invalidation;

    #[test]
    fn a_replacement_scan_invalidates_immediately_and_a_rerun_defers() {
        assert_eq!(Invalidation::on_scan_start(false), Invalidation::all());
        assert_eq!(Invalidation::on_scan_start(true), Invalidation::none());
        assert!(!Invalidation::on_scan_start(true).any());
        assert_eq!(Invalidation::on_targeted_commit(), Invalidation::all());
    }

    #[test]
    fn new_issues_drop_only_the_issue_derived_projections() {
        let invalidation = Invalidation::on_issue_projection();
        assert!(invalidation.fix_plan && invalidation.prioritization);
        assert!(!invalidation.report && !invalidation.analyses);
    }
}
