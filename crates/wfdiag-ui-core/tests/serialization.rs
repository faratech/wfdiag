use serde_json::json;
use wfdiag_ui_core::{
    ActionItemRun, ActionItemStatus, ActionPreview, ActionProposal, ActionRunStatus, ActionStatus,
    ApprovalScope, ChatDelta, ChatEvent, ChatFinishReason, ChatProposal, DiagnosticTaskResult,
    FixCompletionStatus, FixResult, ProviderExecutionClass, ProviderUse, QuickScanRequest,
    QuickScanSource, RemediationStepResult, RemediationStepStatus, RemediationSummary,
    RemediationTier, ReportDone, ReportEvent, SystemStats, TaskProgress, TaskProgressStatus,
    UiEvent,
};

#[test]
fn task_progress_wire_shape_is_stable() {
    let event = UiEvent::TaskProgress(TaskProgress {
        session_id: "scan-7".into(),
        task_id: "cpu".into(),
        status: TaskProgressStatus::Running,
        task_name: Some("CPU information".into()),
        success: None,
    });

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "task_progress",
            "payload": {
                "session_id": "scan-7",
                "task_id": "cpu",
                "status": "running",
                "task_name": "CPU information"
            }
        })
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn diagnostic_result_wire_shape_is_stable() {
    let shared = std::sync::Arc::new(wfdiag_native_issues::TaskResult {
        success: false,
        output: "partial evidence".into(),
        error: Some("counter unavailable".into()),
        duration_ms: 42,
    });
    let event = UiEvent::DiagnosticResult(DiagnosticTaskResult::new(
        "scan-7",
        "cpu",
        std::sync::Arc::clone(&shared),
    ));

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "diagnostic_result",
            "payload": {
                "session_id": "scan-7",
                "task_id": "cpu",
                "success": false,
                "output": "partial evidence",
                "error": "counter unavailable",
                "duration_ms": 42
            }
        })
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
    let UiEvent::DiagnosticResult(result) = &event else {
        unreachable!("constructed a diagnostic result")
    };
    assert!(std::sync::Arc::ptr_eq(&result.result, &shared));
    assert!(event.is_lossless());
}

#[test]
fn diagnostic_result_without_error_keeps_the_omitted_wire_field() {
    let event = UiEvent::DiagnosticResult(DiagnosticTaskResult::new(
        "scan-8",
        "memory",
        std::sync::Arc::new(wfdiag_native_issues::TaskResult {
            success: true,
            output: "ok".into(),
            error: None,
            duration_ms: 7,
        }),
    ));

    let value = serde_json::to_value(&event).unwrap();
    assert!(value["payload"].get("error").is_none());
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn nested_chat_wire_shape_uses_camel_case_payloads() {
    let event = UiEvent::Chat(ChatEvent::Delta(ChatDelta {
        session_id: "chat-1".into(),
        message_id: "message-2".into(),
        text: "Hello".into(),
    }));

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "chat",
            "payload": {
                "kind": "delta",
                "payload": {
                    "sessionId": "chat-1",
                    "messageId": "message-2",
                    "text": "Hello"
                }
            }
        })
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn action_and_quick_scan_variants_round_trip() {
    let events = vec![
        UiEvent::ActionStatus(ActionStatus {
            run_id: "run-1".into(),
            proposal_id: "proposal-1".into(),
            authorization_id: "authorization-1".into(),
            status: ActionRunStatus::Cancelled,
            actions: Vec::new(),
            current_index: Some(0),
            approved_at_ms: 100,
            completed_at_ms: Some(200),
            scan_fingerprint: "scan".into(),
            catalog_fingerprint: "catalog".into(),
        }),
        UiEvent::QuickScan(QuickScanRequest {
            request_id: "request-1".into(),
            requested_at_ms: 300,
            source: QuickScanSource::Tray,
        }),
    ];

    for event in events {
        let encoded = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<UiEvent>(&encoded).unwrap(), event);
    }
}

#[test]
fn nested_proposal_keeps_remediation_catalog_fields_in_snake_case() {
    let event = UiEvent::Chat(ChatEvent::Proposal(ChatProposal {
        session_id: "chat-1".into(),
        message_id: "message-2".into(),
        proposal: ActionProposal {
            proposal_id: "proposal-3".into(),
            approval_scope: ApprovalScope::Exact,
            actions: vec![ActionPreview {
                remediation: RemediationSummary {
                    id: "repair-system-files".into(),
                    label: "Repair system files".into(),
                    description: "Runs the catalog-backed repair".into(),
                    tier: RemediationTier::Repair,
                    admin_required: true,
                    requires_restart: true,
                    long_running: true,
                    maintenance: false,
                    batch_eligible: false,
                    cancellable: true,
                },
                issue_id: Some("system-files".into()),
                steps: vec!["Run DISM".into(), "Run SFC".into()],
            }],
            scan_fingerprint: "scan-fingerprint".into(),
            catalog_fingerprint: "catalog-fingerprint".into(),
            created_at_ms: 100,
            expires_at_ms: 200,
        },
    }));

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "chat",
            "payload": {
                "kind": "proposal",
                "payload": {
                    "sessionId": "chat-1",
                    "messageId": "message-2",
                    "proposal": {
                        "proposalId": "proposal-3",
                        "approvalScope": "exact",
                        "actions": [{
                            "remediation": {
                                "id": "repair-system-files",
                                "label": "Repair system files",
                                "description": "Runs the catalog-backed repair",
                                "tier": "repair",
                                "admin_required": true,
                                "requires_restart": true,
                                "long_running": true,
                                "maintenance": false,
                                "batch_eligible": false,
                                "cancellable": true
                            },
                            "issueId": "system-files",
                            "steps": ["Run DISM", "Run SFC"]
                        }],
                        "scanFingerprint": "scan-fingerprint",
                        "catalogFingerprint": "catalog-fingerprint",
                        "createdAtMs": 100,
                        "expiresAtMs": 200
                    }
                }
            }
        })
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn nested_action_status_keeps_fix_result_fields_in_snake_case() {
    let event = UiEvent::ActionStatus(ActionStatus {
        run_id: "run-1".into(),
        proposal_id: "proposal-1".into(),
        authorization_id: "authorization-1".into(),
        status: ActionRunStatus::Succeeded,
        actions: vec![ActionItemRun {
            remediation_id: "repair-system-files".into(),
            label: "Repair system files".into(),
            cancellable: true,
            status: ActionItemStatus::Succeeded,
            result: Some(FixResult {
                success: true,
                message: "Repair completed".into(),
                actions_taken: vec!["Ran SFC".into()],
                requires_restart: true,
                completion_status: FixCompletionStatus::Succeeded,
                steps: vec![RemediationStepResult {
                    action: "Run SFC".into(),
                    status: RemediationStepStatus::Succeeded,
                    detail: Some("No integrity violations".into()),
                }],
            }),
            error: None,
        }],
        current_index: Some(0),
        approved_at_ms: 100,
        completed_at_ms: Some(200),
        scan_fingerprint: "scan-fingerprint".into(),
        catalog_fingerprint: "catalog-fingerprint".into(),
    });

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "action_status",
            "payload": {
                "runId": "run-1",
                "proposalId": "proposal-1",
                "authorizationId": "authorization-1",
                "status": "succeeded",
                "actions": [{
                    "remediationId": "repair-system-files",
                    "label": "Repair system files",
                    "cancellable": true,
                    "status": "succeeded",
                    "result": {
                        "success": true,
                        "message": "Repair completed",
                        "actions_taken": ["Ran SFC"],
                        "requires_restart": true,
                        "completion_status": "succeeded",
                        "steps": [{
                            "action": "Run SFC",
                            "status": "succeeded",
                            "detail": "No integrity violations"
                        }]
                    }
                }],
                "currentIndex": 0,
                "approvedAtMs": 100,
                "completedAtMs": 200,
                "scanFingerprint": "scan-fingerprint",
                "catalogFingerprint": "catalog-fingerprint"
            }
        })
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn system_stats_round_trip_without_backend_only_process_data() {
    let event = UiEvent::SystemStats(SystemStats {
        cpu_utilization: 12.5,
        per_cpu_utilization: vec![10.0, 15.0],
        cpu_frequency: 4_200,
        memory_total_gb: 32.0,
        memory_used_gb: 8.0,
        memory_available_gb: 24.0,
        memory_utilization: 25.0,
        swap_total_gb: 4.0,
        swap_used_gb: 0.5,
        swap_utilization: 12.5,
        storage_used_percent: 40.0,
        disk_utilization: 40.0,
        disk_read_bytes: 100,
        disk_write_bytes: 200,
        disks: Vec::new(),
        network_upload_kb: 1.0,
        network_download_kb: 2.0,
        gpu_available: false,
        gpu_name: None,
        gpu_utilization: None,
        gpu_memory_used_mb: 0.0,
        gpu_memory_total_mb: 0.0,
        npu_available: false,
        npu_name: None,
        npu_utilization: None,
        npu_memory_used_mb: 0.0,
        npu_memory_total_mb: 0.0,
        top_processes: Vec::new(),
        timestamp: 123,
    });

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["type"], "system_stats");
    assert_eq!(value["payload"]["cpu_utilization"], 12.5);
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}

#[test]
fn report_terminal_wire_shape_round_trips_provider_metadata() {
    let event = UiEvent::Report(ReportEvent::Done(ReportDone {
        report_id: "report-1".into(),
        finish_reason: ChatFinishReason::Stop,
        provider: "openai".into(),
        provider_use: ProviderUse {
            provider_id: "openai".into(),
            execution_class: ProviderExecutionClass::ApiCloud,
            fallback_from: Some("phi_silica".into()),
            requested_model: Some("gpt-5".into()),
            actual_models: vec!["gpt-5.2".into()],
        },
    }));

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["payload"]["kind"], "done");
    assert_eq!(
        value["payload"]["payload"]["providerUse"]["fallbackFrom"],
        "phi_silica"
    );
    assert_eq!(serde_json::from_value::<UiEvent>(value).unwrap(), event);
}
