use super::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const CURRENT: &str = "2.5.8";
const RELEASE_URL: &str = "https://github.com/faratech/wfdiag/releases/tag/v2.6.0";

#[derive(Debug)]
struct FakeHttp {
    calls: AtomicUsize,
    requests: Mutex<Vec<ReleaseRequest>>,
    response: Result<ReleaseResponse, String>,
}

impl FakeHttp {
    fn success(tag: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            response: Ok(ReleaseResponse {
                status: 200,
                body: release_json(tag, false, false, Some("notes")),
            }),
        }
    }

    fn with_response(response: Result<ReleaseResponse, String>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            response,
        }
    }
}

impl ReleaseHttp for FakeHttp {
    fn fetch_latest(&self, request: &ReleaseRequest) -> Result<ReleaseResponse, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(request.clone());
        self.response.clone()
    }
}

#[derive(Debug)]
struct FakeSignature {
    identity_calls: AtomicUsize,
    signature_calls: AtomicUsize,
    has_identity: bool,
    signature: Result<PackageSignature, String>,
}

impl FakeSignature {
    fn unpackaged() -> Self {
        Self {
            identity_calls: AtomicUsize::new(0),
            signature_calls: AtomicUsize::new(0),
            has_identity: false,
            signature: Ok(PackageSignature::Other),
        }
    }
}

impl SignatureProvider for FakeSignature {
    fn has_package_identity(&self) -> bool {
        self.identity_calls.fetch_add(1, Ordering::Relaxed);
        self.has_identity
    }

    fn signature(&self) -> Result<PackageSignature, String> {
        self.signature_calls.fetch_add(1, Ordering::Relaxed);
        self.signature.clone()
    }
}

#[derive(Debug)]
struct FakeVersion {
    calls: AtomicUsize,
    version: Version,
}

impl FakeVersion {
    fn current() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            version: Version::parse(CURRENT).unwrap(),
        }
    }
}

impl CurrentVersionProvider for FakeVersion {
    fn current_version(&self) -> Version {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.version.clone()
    }
}

fn release_json(tag: &str, draft: bool, prerelease: bool, body: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "tag_name": tag,
        "html_url": RELEASE_URL,
        "published_at": "2026-08-30T00:00:00Z",
        "body": body,
        "draft": draft,
        "prerelease": prerelease,
    }))
    .unwrap()
}

fn service(
    http: Arc<FakeHttp>,
    signature: Arc<FakeSignature>,
    version: Arc<FakeVersion>,
    debug: bool,
) -> UpdateService {
    UpdateService::new(http, signature, version, debug)
}

#[test]
fn static_version_constructor_accepts_the_shipping_version() {
    let provider = StaticCurrentVersion::parse("2.5.8").unwrap();
    assert_eq!(provider.current_version(), Version::parse("2.5.8").unwrap());
}

#[test]
fn shipping_string_constructor_rejects_invalid_version_without_starting_io() {
    assert!(UpdateService::shipping_from_str("not-a-version", false).is_err());
}

#[test]
fn debug_build_short_circuits_every_provider() {
    let http = Arc::new(FakeHttp::success("v9.0.0"));
    let signature = Arc::new(FakeSignature::unpackaged());
    let version = Arc::new(FakeVersion::current());
    assert_eq!(
        service(http.clone(), signature.clone(), version.clone(), true).check_outcome(),
        UpdateOutcome::Silent
    );
    assert_eq!(signature.identity_calls.load(Ordering::Relaxed), 0);
    assert_eq!(signature.signature_calls.load(Ordering::Relaxed), 0);
    assert_eq!(version.calls.load(Ordering::Relaxed), 0);
    assert_eq!(http.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn store_install_is_silent_and_never_reads_version_or_network() {
    let http = Arc::new(FakeHttp::success("v9.0.0"));
    let signature = Arc::new(FakeSignature {
        identity_calls: AtomicUsize::new(0),
        signature_calls: AtomicUsize::new(0),
        has_identity: true,
        signature: Ok(PackageSignature::Store),
    });
    let version = Arc::new(FakeVersion::current());
    assert_eq!(
        service(http.clone(), signature.clone(), version.clone(), false).check_outcome(),
        UpdateOutcome::Silent
    );
    assert_eq!(signature.identity_calls.load(Ordering::Relaxed), 1);
    assert_eq!(signature.signature_calls.load(Ordering::Relaxed), 1);
    assert_eq!(version.calls.load(Ordering::Relaxed), 0);
    assert_eq!(http.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn signature_api_failure_fails_closed_as_store() {
    let signature = FakeSignature {
        identity_calls: AtomicUsize::new(0),
        signature_calls: AtomicUsize::new(0),
        has_identity: true,
        signature: Err("package API failed".to_string()),
    };
    assert!(is_store_install(&signature));
    assert_eq!(signature.signature_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn unpackaged_process_does_not_query_signature_kind() {
    let signature = FakeSignature::unpackaged();
    assert!(!is_store_install(&signature));
    assert_eq!(signature.identity_calls.load(Ordering::Relaxed), 1);
    assert_eq!(signature.signature_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn developer_identity_stays_on_github_channel() {
    let http = Arc::new(FakeHttp::success("v2.6.0"));
    let signature = Arc::new(FakeSignature {
        identity_calls: AtomicUsize::new(0),
        signature_calls: AtomicUsize::new(0),
        has_identity: true,
        signature: Ok(PackageSignature::Other),
    });
    let update = service(
        http.clone(),
        signature,
        Arc::new(FakeVersion::current()),
        false,
    )
    .check_outcome()
    .into_available()
    .unwrap();
    assert_eq!(update.version, "2.6.0");
    assert_eq!(http.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn request_contract_matches_shipping_endpoint_headers_and_timeout() {
    let http = Arc::new(FakeHttp::success("v2.6.0"));
    let update = service(
        http.clone(),
        Arc::new(FakeSignature::unpackaged()),
        Arc::new(FakeVersion::current()),
        false,
    )
    .check_outcome()
    .into_available()
    .unwrap();
    assert_eq!(update.version, "2.6.0");
    let requests = http.requests.lock().unwrap();
    assert_eq!(
        requests.as_slice(),
        &[ReleaseRequest {
            url: RELEASES_LATEST_URL,
            user_agent: "wfdiag/2.5.8".to_string(),
            accept: GITHUB_JSON_ACCEPT,
            timeout: Duration::from_secs(10),
        }]
    );
}

#[test]
fn newer_release_maps_the_complete_contract() {
    let body = "é".repeat(301);
    let http = Arc::new(FakeHttp::with_response(Ok(ReleaseResponse {
        status: 200,
        body: release_json("  v2.6.0  ", false, false, Some(&body)),
    })));
    let update = service(
        http,
        Arc::new(FakeSignature::unpackaged()),
        Arc::new(FakeVersion::current()),
        false,
    )
    .check_outcome()
    .into_available()
    .unwrap();
    assert_eq!(update.version, "2.6.0");
    assert_eq!(update.html_url, RELEASE_URL);
    assert_eq!(update.published_at.as_deref(), Some("2026-08-30T00:00:00Z"));
    let excerpt = update.notes_excerpt.unwrap();
    assert_eq!(excerpt.chars().count(), 300);
    assert_eq!(excerpt.len(), 600);
}

#[test]
fn same_older_malformed_draft_and_prerelease_are_silent() {
    for (tag, draft, prerelease) in [
        ("v2.5.8", false, false),
        ("v2.5.7", false, false),
        ("nightly", false, false),
        ("v9.0.0", true, false),
        ("v9.0.0", false, true),
    ] {
        let http = Arc::new(FakeHttp::with_response(Ok(ReleaseResponse {
            status: 200,
            body: release_json(tag, draft, prerelease, Some("notes")),
        })));
        assert_eq!(
            service(
                http,
                Arc::new(FakeSignature::unpackaged()),
                Arc::new(FakeVersion::current()),
                false,
            )
            .check_outcome(),
            UpdateOutcome::UpToDate,
            "case {tag} draft={draft} prerelease={prerelease}"
        );
    }
}

#[test]
fn transport_http_and_json_failures_are_distinguishable_from_up_to_date() {
    let cases = [
        (
            FakeHttp::with_response(Err("offline".to_string())),
            UpdateFailure::Transport("offline".to_string()),
        ),
        (
            FakeHttp::with_response(Ok(ReleaseResponse {
                status: 403,
                body: b"rate limited".to_vec(),
            })),
            UpdateFailure::Status(403),
        ),
    ];
    for (http, expected) in cases {
        let outcome = service(
            Arc::new(http),
            Arc::new(FakeSignature::unpackaged()),
            Arc::new(FakeVersion::current()),
            false,
        )
        .check_outcome();
        assert_eq!(outcome, UpdateOutcome::Failed(expected));
        assert!(outcome.available().is_none());
    }

    // The parse diagnostic is serde's, so match the variant rather than text.
    let outcome = service(
        Arc::new(FakeHttp::with_response(Ok(ReleaseResponse {
            status: 200,
            body: b"not json".to_vec(),
        }))),
        Arc::new(FakeSignature::unpackaged()),
        Arc::new(FakeVersion::current()),
        false,
    )
    .check_outcome();
    assert!(
        matches!(outcome.failure(), Some(UpdateFailure::Parse(_))),
        "{outcome:?}"
    );
}

#[test]
fn a_failed_check_is_never_confused_with_an_available_release() {
    let failed = UpdateOutcome::Failed(UpdateFailure::Status(403));
    assert!(failed.available().is_none());
    assert!(failed.clone().into_available().is_none());
    assert!(failed.failure().is_some());
    assert!(UpdateOutcome::UpToDate.failure().is_none());
    assert!(UpdateOutcome::Silent.failure().is_none());
    assert_eq!(
        UpdateFailure::Status(403).to_string(),
        "update request returned HTTP status 403"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_is_nonblocking_and_returns_typed_result() {
    let runtime = NativeUpdateRuntime::start(service(
        Arc::new(FakeHttp::success("v2.6.0")),
        Arc::new(FakeSignature::unpackaged()),
        Arc::new(FakeVersion::current()),
        false,
    ))
    .unwrap();
    let reply = runtime.request_check().unwrap();
    let update = tokio::time::timeout(Duration::from_secs(2), reply)
        .await
        .unwrap()
        .unwrap()
        .into_available()
        .unwrap();
    assert_eq!(update.version, "2.6.0");
}
