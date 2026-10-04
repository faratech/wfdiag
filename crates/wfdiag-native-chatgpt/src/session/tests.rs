use super::*;
use crate::{AuthorizationUrl, BrowserLauncher, Clock, HttpClient, HttpFuture, SystemClock};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;
use sha2::Digest;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use wfdiag_native_settings::{MemoryProtectedDocumentStorage, ProtectedDocumentStorage};

#[derive(Default)]
struct Browser {
    queries: Mutex<Vec<HashMap<String, String>>>,
    skip_callback: AtomicBool,
}
impl BrowserLauncher for Browser {
    fn open(&self, target: &AuthorizationUrl) -> Result<(), String> {
        let url = url::Url::parse(target.as_str()).unwrap();
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let redirect = query["redirect_uri"].clone();
        let state = query["state"].clone();
        let client = if query["client_id"] == "dynamic_agent_client" {
            format!("issued-{}", self.queries.lock().unwrap().len())
        } else {
            query["client_id"].clone()
        };
        self.queries.lock().unwrap().push(query);
        if !self.skip_callback.load(Ordering::Acquire) {
            tokio::spawn(async move {
                // First send an unrelated callback. It must not terminate sign-in.
                let callback = format!("{redirect}?state={state}&code=fixture&client_id={client}");
                let client = reqwest::Client::new();
                let _ = client.get(&redirect).send().await.unwrap();
                client.get(callback).send().await.unwrap();
            });
        }
        Ok(())
    }
}
type HttpCall = (String, Vec<(&'static str, String)>);
struct FixtureHttp {
    browser: Arc<Browser>,
    scope: Mutex<String>,
    subject: Mutex<String>,
    calls: Mutex<Vec<HttpCall>>,
    renewals: AtomicUsize,
    failure: Mutex<Option<Failure>>,
    wrong_audience: AtomicBool,
    revoke_failure: AtomicBool,
    revoke_wait: AtomicBool,
    models: Mutex<Value>,
}
impl FixtureHttp {
    fn new(browser: Arc<Browser>) -> Self {
        Self {
            browser,
            scope: Mutex::new(crate::SCOPES.into()),
            subject: Mutex::new("subject-one".into()),
            calls: Mutex::new(Vec::new()),
            renewals: AtomicUsize::new(0),
            failure: Mutex::new(None),
            wrong_audience: AtomicBool::new(false),
            revoke_failure: AtomicBool::new(false),
            revoke_wait: AtomicBool::new(false),
            models: Mutex::new(
                json!({"models":[{"slug":"first","display_name":"First model","visibility":"list"},{"slug":"hidden","display_name":"Hidden","visibility":"hidden"},{"slug":"second","display_name":"Second model","visibility":"list"}]}),
            ),
        }
    }
    fn tokens(&self, client: &str, nonce: &str) -> Value {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("fixture".into());
        let claims = json!({"iss":"https://auth.openai.com","aud":if self.wrong_audience.load(Ordering::Acquire) { "attacker" } else { client },"exp":SystemClock.now()+3600,"nonce":nonce,"sub":*self.subject.lock().unwrap(),"email":"same@example.test"});
        let identity = encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_der(include_bytes!(
                "../../tests/fixtures/identity-test-key.der"
            )),
        )
        .unwrap();
        json!({"access_token":"access-fixture","refresh_token":"refresh-fixture","id_token":identity,"scope":*self.scope.lock().unwrap(),"expires_in":3600,"token_type":"Bearer","earliest_refresh_at":0})
    }
}
impl HttpClient for FixtureHttp {
    fn get<'a>(&'a self, endpoint: &'a str, _: Option<&'a Secret>) -> HttpFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((endpoint.into(), Vec::new()));
            match endpoint {
                DISCOVERY_URL => Ok(
                    json!({"issuer":"https://auth.openai.com","jwks_uri":"https://auth.openai.com/.well-known/jwks.json","revocation_endpoint":"https://auth.openai.com/api/accounts/oauth/revoke"}),
                ),
                "https://auth.openai.com/.well-known/jwks.json" => Ok(serde_json::from_str(
                    include_str!("../../tests/fixtures/jwks.json"),
                )
                .unwrap()),
                "https://api.openai.com/v1/models" => Ok(self.models.lock().unwrap().clone()),
                _ => panic!("unexpected fixture endpoint"),
            }
        })
    }
    fn post_form<'a>(
        &'a self,
        endpoint: &'a str,
        form: Vec<(&'static str, String)>,
    ) -> HttpFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((endpoint.into(), form.clone()));
            if endpoint.ends_with("/revoke") {
                if self.revoke_wait.load(Ordering::Acquire) {
                    std::future::pending::<()>().await;
                }
                return if self.revoke_failure.load(Ordering::Acquire) {
                    Err(Failure::network())
                } else {
                    Ok(Value::Null)
                };
            }
            let fields: HashMap<_, _> = form.into_iter().collect();
            assert_ne!(fields["client_id"], "dynamic_agent_client");
            assert_eq!(fields["resource"], API_BASE);
            if fields["grant_type"] == "authorization_code" {
                let query = self.browser.queries.lock().unwrap().last().unwrap().clone();
                assert_eq!(fields["redirect_uri"], query["redirect_uri"]);
                assert_eq!(query["code_challenge_method"], "S256");
                assert_eq!(
                    query["code_challenge"],
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .encode(sha2::Sha256::digest(fields["code_verifier"].as_bytes()))
                );
                return Ok(self.tokens(&fields["client_id"], &query["nonce"]));
            }
            self.renewals.fetch_add(1, Ordering::AcqRel);
            tokio::time::sleep(Duration::from_millis(10)).await;
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            assert_eq!(fields["refresh_token"], "refresh-fixture");
            assert!(!fields.contains_key("scope"));
            Ok(
                json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"token_type":"Bearer"}),
            )
        })
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        1000
    }
}
fn fixture() -> (
    Arc<ChatGptService>,
    Arc<FixtureHttp>,
    Arc<MemoryProtectedDocumentStorage>,
) {
    let browser = Arc::new(Browser::default());
    let http = Arc::new(FixtureHttp::new(Arc::clone(&browser)));
    let storage = Arc::new(MemoryProtectedDocumentStorage::default());
    let service = Arc::new(ChatGptService::new(ChatGptPorts {
        storage: storage.clone(),
        http: http.clone(),
        browser,
        clock: Arc::new(FixedClock),
    }));
    (service, http, storage)
}
async fn connect(service: &Arc<ChatGptService>, account: Option<AccountId>, consent: bool) {
    tokio::time::timeout(
        Duration::from_secs(5),
        service.execute(
            AccountCommand::Connect { account, consent },
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
}
fn update_document(storage: &dyn ProtectedDocumentStorage, update: impl FnOnce(&mut Document)) {
    let guard = storage.try_lock().unwrap().unwrap();
    let mut document = read(guard.as_ref()).unwrap();
    update(&mut document);
    save(guard.as_ref(), &document).unwrap();
}

#[tokio::test]
async fn browser_pkce_identity_models_and_same_email_registrations() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    let first = service.snapshot().active.unwrap();
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    assert_eq!(lease.model, "first");
    assert_eq!(service.snapshot().selected().unwrap().models.len(), 2);
    service
        .execute(AccountCommand::DismissWelcome, CancellationToken::new())
        .await
        .unwrap();
    connect(&service, Some(first.clone()), false).await;
    assert!(!service.snapshot().welcome);
    {
        let queries = http.browser.queries.lock().unwrap();
        assert_eq!(queries[0]["agent_name_hint"], "WindowsForum Diagnostics");
        assert_eq!(queries[1]["client_id"], "issued-0");
        assert!(!queries[1].contains_key("agent_name_hint"));
        assert!(queries[1].contains_key("id_token_hint"));
        assert_eq!(
            queries[0]["ext_agent_host_id"],
            queries[1]["ext_agent_host_id"]
        );
        assert_ne!(queries[0]["state"], queries[1]["state"]);
        assert_ne!(queries[0]["nonce"], queries[1]["nonce"]);
    }
    *http.subject.lock().unwrap() = "subject-two".into();
    connect(&service, None, false).await;
    assert!(lease.cancel.is_cancelled());
    let snapshot = service.snapshot();
    assert_eq!(snapshot.accounts.len(), 2);
    assert_ne!(snapshot.accounts[0].id, snapshot.accounts[1].id);
    assert_ne!(snapshot.accounts[0].label, snapshot.accounts[1].label);
    assert_eq!(snapshot.accounts[0].email, snapshot.accounts[1].email);
    assert!(!format!("{snapshot:?}").contains("access-fixture"));
    let guard = storage.try_lock().unwrap().unwrap();
    let document = read(guard.as_ref()).unwrap();
    assert_ne!(
        document.accounts[0].client_id,
        document.accounts[1].client_id
    );
}

#[tokio::test]
async fn missing_scope_retains_identity_and_explicit_consent_reuses_registration() {
    let (service, http, _) = fixture();
    *http.scope.lock().unwrap() = "openid profile email offline_access resource.invoke".into();
    connect(&service, None, false).await;
    assert_eq!(
        service.snapshot().selected().unwrap().state,
        AccountState::ConsentRequired
    );
    assert!(service.lease(&CancellationToken::new()).await.is_err());
    assert!(
        !http
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(u, _)| u.ends_with("/models"))
    );
    *http.scope.lock().unwrap() = crate::SCOPES.into();
    connect(&service, service.snapshot().active, true).await;
    assert_eq!(
        service.snapshot().selected().unwrap().state,
        AccountState::Ready
    );
    assert_eq!(
        http.browser.queries.lock().unwrap().last().unwrap()["prompt"],
        "consent"
    );
}

#[tokio::test]
async fn catalog_replacing_selected_model_cancels_old_session_and_preserves_server_order() {
    let (service, http, _) = fixture();
    connect(&service, None, false).await;
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    let generation = service.snapshot().selected().unwrap().generation;
    *http.models.lock().unwrap() = json!({"models":[{"slug":"second","display_name":"Second","visibility":"list"},{"slug":"third","display_name":"Third","visibility":"list"}]});
    let models = service
        .refresh_models(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.slug.as_str())
            .collect::<Vec<_>>(),
        ["second", "third"]
    );
    let snapshot = service.snapshot();
    assert_eq!(
        snapshot.selected().unwrap().model.as_deref(),
        Some("second")
    );
    assert!(snapshot.selected().unwrap().generation > generation);
    assert!(lease.cancel.is_cancelled());
    assert!(service.validate_lease(&lease).await.is_err());
    assert!(service.access_token(&lease).await.is_err());
}

#[tokio::test]
async fn invalid_identity_never_replaces_active_account_and_keeps_registration_for_retry() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    let original = service.snapshot().active;
    http.wrong_audience.store(true, Ordering::Release);
    assert!(
        service
            .execute(
                AccountCommand::Connect {
                    account: None,
                    consent: false
                },
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(service.snapshot().active, original);
    let guard = storage.try_lock().unwrap().unwrap();
    let document = read(guard.as_ref()).unwrap();
    assert_eq!(document.accounts.len(), 2);
    assert!(document.accounts[1].tokens.is_none());
    assert_eq!(document.accounts[1].client_id, "issued-1");
}

#[tokio::test]
async fn refresh_is_serialized_across_owners_and_rotation_is_persisted() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    update_document(storage.as_ref(), |d| {
        d.accounts[0].tokens.as_mut().unwrap().expires_at = 1000;
    });
    let second = Arc::new(ChatGptService::new(ChatGptPorts {
        storage: storage.clone(),
        http: http.clone(),
        browser: http.browser.clone(),
        clock: Arc::new(FixedClock),
    }));
    let first_lease = service.lease(&CancellationToken::new()).await.unwrap();
    let second_lease = second.lease(&CancellationToken::new()).await.unwrap();
    let (one, two) = tokio::join!(
        service.access_token(&first_lease),
        second.access_token(&second_lease)
    );
    assert_eq!(one.unwrap().expose(), "rotated-access");
    assert_eq!(two.unwrap().expose(), "rotated-access");
    assert_eq!(http.renewals.load(Ordering::Acquire), 1);
    let guard = storage.try_lock().unwrap().unwrap();
    let doc = read(guard.as_ref()).unwrap();
    assert_eq!(
        doc.accounts[0].tokens.as_ref().unwrap().refresh.expose(),
        "rotated-refresh"
    );
}

#[tokio::test]
async fn temporary_refresh_retains_tokens_terminal_refresh_clears_only_tokens() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    update_document(storage.as_ref(), |d| {
        d.accounts[0].tokens.as_mut().unwrap().expires_at = 1000;
    });
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    *http.failure.lock().unwrap() = Some(Failure::network());
    assert!(service.access_token(&lease).await.is_err());
    assert_eq!(
        read(storage.try_lock().unwrap().unwrap().as_ref())
            .unwrap()
            .accounts[0]
            .tokens
            .as_ref()
            .unwrap()
            .refresh
            .expose(),
        "refresh-fixture"
    );
    *http.failure.lock().unwrap() = Some(Failure::from_response(
        400,
        &json!({"error":"invalid_grant"}),
        None,
    ));
    assert!(service.access_token(&lease).await.is_err());
    service
        .record_failure(&lease, Failure::local("wrapper"))
        .await;
    assert_eq!(
        service.snapshot().selected().unwrap().state,
        AccountState::ReconnectRequired
    );
    let document = read(storage.try_lock().unwrap().unwrap().as_ref()).unwrap();
    assert!(document.accounts[0].tokens.is_none());
    assert_eq!(document.accounts[0].client_id, "issued-0");
}

#[tokio::test]
async fn earliest_refresh_and_quota_prevent_requests_without_erasing_credentials() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    update_document(storage.as_ref(), |d| {
        let t = d.accounts[0].tokens.as_mut().unwrap();
        t.expires_at = 1000;
        t.earliest_refresh_at = 1100;
    });
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    assert!(service.access_token(&lease).await.is_err());
    assert_eq!(http.renewals.load(Ordering::Acquire), 0);
    service.record_failure(&lease, Failure::from_response(429, &json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"never expose this"}}), Some("req_test".into()))).await;
    assert!(service.lease(&CancellationToken::new()).await.is_err());
    let document = read(storage.try_lock().unwrap().unwrap().as_ref()).unwrap();
    assert!(document.accounts[0].tokens.is_some());
    assert!(
        !service
            .snapshot()
            .selected()
            .unwrap()
            .failure
            .as_ref()
            .unwrap()
            .to_string()
            .contains("never expose")
    );
}

#[tokio::test]
async fn signout_clears_locally_when_revocation_fails_and_keeps_host() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    let before = read(storage.try_lock().unwrap().unwrap().as_ref()).unwrap();
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    http.revoke_failure.store(true, Ordering::Release);
    service
        .execute(
            AccountCommand::SignOut(before.active.unwrap()),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(lease.cancel.is_cancelled());
    assert!(service.snapshot().active.is_none());
    let after = read(storage.try_lock().unwrap().unwrap().as_ref()).unwrap();
    assert_eq!(before.host_id, after.host_id);
    assert_eq!(after.accounts[0].client_id, "issued-0");
    assert!(after.accounts[0].tokens.is_none());
    assert!(
        service
            .take_events()
            .iter()
            .any(|e| matches!(e, AccountEvent::Notice(_)))
    );
}

#[tokio::test]
async fn cancelled_revocation_keeps_local_signout_and_reports_remote_uncertainty() {
    let (service, http, storage) = fixture();
    connect(&service, None, false).await;
    http.revoke_wait.store(true, Ordering::Release);
    let account = service.snapshot().active.unwrap();
    let cancel = CancellationToken::new();
    let task_service = service.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        task_service
            .execute(AccountCommand::SignOut(account), task_cancel)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if http
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(endpoint, _)| endpoint.ends_with("/revoke"))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(service.snapshot().active.is_none());
    assert!(!service.snapshot().busy);
    let document = read(storage.try_lock().unwrap().unwrap().as_ref()).unwrap();
    assert!(document.accounts[0].tokens.is_none());
    assert!(service.take_events().iter().any(
        |event| matches!(event, AccountEvent::Notice(message) if message.contains("not confirmed"))
    ));
}

#[test]
fn recovery_preserves_error_shapes_without_copying_untrusted_detail() {
    use crate::ErrorBodyShape;
    let admission = Failure::from_response(
        403,
        &json!({"detail":"private diagnostic"}),
        Some("req_test".into()),
    );
    assert_eq!(admission.body_shape, ErrorBodyShape::DirectAdmission);
    assert_eq!(admission.status, Some(403));
    assert_eq!(admission.state, AccountState::Restricted);
    assert!(!format!("{admission:?}").contains("private diagnostic"));
    let oauth = Failure::from_response(
        400,
        &json!({"error":"invalid_grant","error_description":"private diagnostic"}),
        None,
    );
    assert_eq!(oauth.body_shape, ErrorBodyShape::OAuth);
    assert_eq!(oauth.state, AccountState::ReconnectRequired);
    let api = Failure::from_response(
        400,
        &json!({"error":{"code":"subscription_sharing_unsupported_capability","param":"tools[0].type"}}),
        None,
    );
    assert_eq!(api.body_shape, ErrorBodyShape::ApiError);
    assert_eq!(api.param.as_deref(), Some("tools[0].type"));
    assert_eq!(api.state, AccountState::Restricted);
}

#[tokio::test]
async fn cancelled_browser_attempt_closes_listener_and_preserves_active_account() {
    let (service, http, _) = fixture();
    connect(&service, None, false).await;
    let active = service.snapshot().active;
    http.browser.skip_callback.store(true, Ordering::Release);
    let cancel = CancellationToken::new();
    let task_service = service.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        task_service
            .execute(
                AccountCommand::Connect {
                    account: None,
                    consent: false,
                },
                task_cancel,
            )
            .await
    });
    while http.browser.queries.lock().unwrap().len() < 2 {
        tokio::task::yield_now().await;
    }
    let redirect = http.browser.queries.lock().unwrap().last().unwrap()["redirect_uri"].clone();
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert_eq!(service.snapshot().active, active);
    assert!(!service.snapshot().busy);
    assert!(reqwest::Client::new().get(redirect).send().await.is_err());
}

#[tokio::test]
async fn identity_validation_rejects_wrong_nonce_issuer_expiry_audience_and_signature() {
    let (service, http, _) = fixture();
    let valid = http.tokens("issued", "nonce");
    assert!(
        crate::oauth::validate_identity(&service.ports, &valid, "issued", "nonce")
            .await
            .is_ok()
    );
    assert!(
        crate::oauth::validate_identity(&service.ports, &valid, "issued", "wrong")
            .await
            .is_err()
    );
    assert!(
        crate::oauth::validate_identity(&service.ports, &valid, "different", "nonce")
            .await
            .is_err()
    );
    for (issuer, expiry) in [
        ("https://attacker.example", SystemClock.now() + 3600),
        ("https://auth.openai.com", 0),
    ] {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("fixture".into());
        let signed = encode(
            &header,
            &json!({"iss":issuer,"aud":"issued","exp":expiry,"nonce":"nonce","sub":"subject"}),
            &EncodingKey::from_rsa_der(include_bytes!(
                "../../tests/fixtures/identity-test-key.der"
            )),
        )
        .unwrap();
        assert!(
            crate::oauth::validate_identity(
                &service.ports,
                &json!({"id_token":signed}),
                "issued",
                "nonce"
            )
            .await
            .is_err()
        );
    }
    let mut corrupted = valid["id_token"].as_str().unwrap().to_string();
    let offset = corrupted.rfind('.').unwrap() + 1;
    corrupted.replace_range(
        offset..=offset,
        if &corrupted[offset..=offset] == "a" {
            "b"
        } else {
            "a"
        },
    );
    assert!(
        crate::oauth::validate_identity(
            &service.ports,
            &json!({"id_token":corrupted}),
            "issued",
            "nonce"
        )
        .await
        .is_err()
    );
}

struct FailingStorage {
    memory: Arc<MemoryProtectedDocumentStorage>,
    fail: Arc<AtomicBool>,
}
struct FailingGuard {
    inner: Box<dyn ProtectedDocumentGuard>,
    fail: Arc<AtomicBool>,
}
impl ProtectedDocumentStorage for FailingStorage {
    fn try_lock(&self) -> Result<Option<Box<dyn ProtectedDocumentGuard>>, String> {
        Ok(self.memory.try_lock()?.map(|inner| {
            Box::new(FailingGuard {
                inner,
                fail: self.fail.clone(),
            }) as Box<dyn ProtectedDocumentGuard>
        }))
    }
}
impl ProtectedDocumentGuard for FailingGuard {
    fn load(&self) -> Result<Option<Zeroizing<String>>, String> {
        self.inner.load()
    }
    fn save(&self, text: &str) -> Result<(), String> {
        if self.fail.load(Ordering::Acquire) {
            Err("fixture disk full".into())
        } else {
            self.inner.save(text)
        }
    }
}
#[tokio::test]
async fn failed_rotation_persistence_never_reuses_old_refresh_token_or_returns_inference_token() {
    let (original, http, memory) = fixture();
    connect(&original, None, false).await;
    update_document(memory.as_ref(), |d| {
        d.accounts[0].tokens.as_mut().unwrap().expires_at = 1000;
    });
    let fail = Arc::new(AtomicBool::new(false));
    let service = Arc::new(ChatGptService::new(ChatGptPorts {
        storage: Arc::new(FailingStorage {
            memory,
            fail: fail.clone(),
        }),
        http: http.clone(),
        browser: http.browser.clone(),
        clock: Arc::new(FixedClock),
    }));
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    fail.store(true, Ordering::Release);
    assert!(service.access_token(&lease).await.is_err());
    assert!(service.access_token(&lease).await.is_err());
    assert_eq!(http.renewals.load(Ordering::Acquire), 1);
    assert_eq!(
        service.snapshot().selected().unwrap().state,
        AccountState::ReconnectRequired
    );
}
#[tokio::test]
async fn cancellation_during_refresh_finishes_atomic_rotation() {
    let (service, http, memory) = fixture();
    connect(&service, None, false).await;
    update_document(memory.as_ref(), |d| {
        d.accounts[0].tokens.as_mut().unwrap().expires_at = 1000;
    });
    let lease = service.lease(&CancellationToken::new()).await.unwrap();
    let owner = service.clone();
    let worker_lease = lease.clone();
    let waiter = tokio::spawn(async move { owner.access_token(&worker_lease).await });
    while http.renewals.load(Ordering::Acquire) == 0 {
        tokio::task::yield_now().await;
    }
    waiter.abort();
    lease.cancel.cancel();
    let guard = loop {
        if let Some(guard) = memory.try_lock().unwrap() {
            break guard;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(
        read(guard.as_ref()).unwrap().accounts[0]
            .tokens
            .as_ref()
            .unwrap()
            .refresh
            .expose(),
        "rotated-refresh"
    );
}
