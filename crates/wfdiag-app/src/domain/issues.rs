//! Two-phase issue detection: commit evidence, then guard the reply.
//!
//! A request id alone cannot decide whether a worker reply is current. Issue
//! detection runs against *committed diagnostic evidence*, and a reply that
//! was computed from evidence which has since been replaced (a new scan, or a
//! rollback to the previous scan) must be dropped even though its request id
//! matches. This tracker therefore matches three things: the request id, the
//! evidence epoch, and the session id of the evidence.

use crate::ids::{Epoch, Epochs, RequestId, RequestIds};
use wfdiag_native_issues::Timestamp;
use wfdiag_native_issues::{IssueDetectionCompleted, IssueDetectionRequest, SharedScanEvidence};

/// The identity captured when a detection request is enqueued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDetection {
    /// The request handed to the worker.
    pub request_id: RequestId,
    /// The evidence epoch the request was built from.
    pub epoch: Epoch,
    /// The session id of that evidence.
    pub session_id: String,
}

/// Why a detection request could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionRefusal {
    /// Nothing has been committed yet.
    NoEvidence,
    /// The request or epoch counter is exhausted.
    IdentityExhausted,
}

/// The committed-evidence tracker.
#[derive(Debug, Default)]
pub struct IssueTracker {
    requests: RequestIds,
    epochs: Epochs,
    committed_epoch: Option<Epoch>,
    session_id: Option<String>,
    evidence: Option<SharedScanEvidence>,
    pending: Option<PendingDetection>,
}

impl IssueTracker {
    /// A tracker with no committed evidence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The evidence epoch currently committed.
    #[must_use]
    pub const fn committed_epoch(&self) -> Option<Epoch> {
        self.committed_epoch
    }

    /// The session id of the committed evidence.
    #[must_use]
    pub fn committed_session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The request currently awaiting a worker reply.
    #[must_use]
    pub const fn pending(&self) -> Option<&PendingDetection> {
        self.pending.as_ref()
    }

    /// Commit one complete authoritative scan as the detection input.
    ///
    /// # Errors
    ///
    /// Returns [`DetectionRefusal::IdentityExhausted`] when the epoch counter
    /// is exhausted; the caller must then stop issue delivery rather than
    /// risk matching a stale reply.
    pub fn commit_evidence(
        &mut self,
        session_id: String,
        evidence: SharedScanEvidence,
    ) -> Result<Epoch, DetectionRefusal> {
        let epoch = self
            .epochs
            .issue()
            .ok_or(DetectionRefusal::IdentityExhausted)?;
        self.committed_epoch = Some(epoch);
        self.session_id = Some(session_id);
        self.evidence = Some(evidence);
        self.pending = None;
        Ok(epoch)
    }

    /// Build the next detection request for the committed evidence.
    ///
    /// Re-running detection advances the request id but deliberately keeps the
    /// evidence epoch: it re-evaluates the same scan.
    ///
    /// # Errors
    ///
    /// Returns [`DetectionRefusal::NoEvidence`] before the first commit and
    /// [`DetectionRefusal::IdentityExhausted`] once request ids run out.
    pub fn prepare(
        &mut self,
        now: Timestamp,
        temp_file_count: Option<usize>,
    ) -> Result<IssueDetectionRequest, DetectionRefusal> {
        let (Some(epoch), Some(session_id), Some(evidence)) = (
            self.committed_epoch,
            self.session_id.clone(),
            self.evidence.as_ref().map(std::sync::Arc::clone),
        ) else {
            return Err(DetectionRefusal::NoEvidence);
        };
        let request_id = self
            .requests
            .issue()
            .ok_or(DetectionRefusal::IdentityExhausted)?;
        self.pending = Some(PendingDetection {
            request_id,
            epoch,
            session_id,
        });
        Ok(IssueDetectionRequest {
            request_id: request_id.get(),
            results: evidence,
            now,
            temp_file_count,
        })
    }

    /// Accept a worker reply only when it belongs to the current committed
    /// evidence. A stale reply leaves a newer pending request intact.
    pub fn accept(&mut self, completion: &IssueDetectionCompleted) -> Option<PendingDetection> {
        let matches = self.pending.as_ref().is_some_and(|pending| {
            pending.request_id.get() == completion.request_id
                && Some(pending.epoch) == self.committed_epoch
                && self.session_id.as_deref() == Some(pending.session_id.as_str())
        });
        if matches { self.pending.take() } else { None }
    }

    /// Forget the in-flight request (worker unavailable, host shutting down).
    pub fn abandon(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{DetectionRefusal, IssueTracker};
    use std::collections::HashMap;
    use std::sync::Arc;
    use wfdiag_native_issues::{IssueDetectionCompleted, TaskResult, Timestamp};

    fn evidence(task: &str) -> wfdiag_native_issues::SharedScanEvidence {
        Arc::new(HashMap::from([(
            task.to_string(),
            Arc::new(TaskResult {
                success: true,
                output: "{}".to_string(),
                error: None,
                duration_ms: 1,
            }),
        )]))
    }

    fn completion(request_id: u64) -> IssueDetectionCompleted {
        IssueDetectionCompleted {
            request_id,
            issues: Vec::new(),
        }
    }

    #[test]
    fn detection_cannot_be_prepared_before_evidence_is_committed() {
        let mut tracker = IssueTracker::new();
        assert_eq!(
            tracker.prepare(Timestamp::from_secs(0), None).unwrap_err(),
            DetectionRefusal::NoEvidence
        );
    }

    #[test]
    fn a_reply_for_the_current_evidence_is_accepted_once() {
        let mut tracker = IssueTracker::new();
        tracker
            .commit_evidence("scan_1".to_string(), evidence("os_info"))
            .expect("first epoch");
        let request = tracker
            .prepare(Timestamp::from_secs(10), Some(3))
            .expect("request");
        assert_eq!(request.request_id, 1);
        assert_eq!(request.temp_file_count, Some(3));
        assert!(tracker.accept(&completion(1)).is_some());
        assert!(
            tracker.accept(&completion(1)).is_none(),
            "a reply is consumed exactly once"
        );
    }

    #[test]
    fn a_reply_computed_from_replaced_evidence_is_dropped() {
        let mut tracker = IssueTracker::new();
        tracker
            .commit_evidence("scan_1".to_string(), evidence("os_info"))
            .expect("first epoch");
        let stale = tracker
            .prepare(Timestamp::from_secs(10), None)
            .expect("request");
        // A new scan commits and its own detection is queued.
        tracker
            .commit_evidence("scan_2".to_string(), evidence("processor"))
            .expect("second epoch");
        let fresh = tracker
            .prepare(Timestamp::from_secs(20), None)
            .expect("request");
        assert!(
            tracker.accept(&completion(stale.request_id)).is_none(),
            "the reply for the replaced evidence is dropped"
        );
        assert!(
            tracker.accept(&completion(fresh.request_id)).is_some(),
            "the newer pending request survives the stale reply"
        );
    }

    #[test]
    fn a_refresh_keeps_the_evidence_epoch_and_advances_the_request_id() {
        let mut tracker = IssueTracker::new();
        let epoch = tracker
            .commit_evidence("scan_1".to_string(), evidence("os_info"))
            .expect("epoch");
        let first = tracker
            .prepare(Timestamp::from_secs(1), None)
            .expect("first");
        let second = tracker
            .prepare(Timestamp::from_secs(2), None)
            .expect("second");
        assert_eq!(first.request_id + 1, second.request_id);
        assert_eq!(tracker.pending().expect("pending").epoch, epoch);
        assert!(
            tracker.accept(&completion(first.request_id)).is_none(),
            "the superseded refresh no longer matches"
        );
        assert!(tracker.accept(&completion(second.request_id)).is_some());
    }
}
