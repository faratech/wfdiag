//! Native Sign in with `ChatGPT`. No vendor CLI credentials are read here.
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]
mod oauth;
mod session;
pub use session::{ChatGptService, SessionLease};
#[derive(Clone, Debug)]
pub struct SessionBinding {
    pub service: Arc<ChatGptService>,
    pub lease: SessionLease,
}
impl PartialEq for SessionBinding {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.service, &other.service)
            && self.lease.fingerprint() == other.lease.fingerprint()
    }
}
impl Eq for SessionBinding {}
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const API_BASE: &str = "https://api.openai.com/v1";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
pub const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub type AccountId = String;
pub type HttpFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'a>>;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop, Default)]
#[serde(transparent)]
pub struct Secret(String);
impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Only the engine can construct this value; its contents never enter diagnostics.
pub struct AuthorizationUrl(url::Url);
impl AuthorizationUrl {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub(crate) fn new(url: url::Url) -> Result<Self, String> {
        if url.scheme() != "https"
            || url.host_str() != Some("auth.openai.com")
            || url.path() != "/api/accounts/authorize"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("Invalid ChatGPT authorization destination".into());
        }
        Ok(Self(url))
    }
}
impl std::fmt::Debug for AuthorizationUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizationUrl([REDACTED])")
    }
}
pub trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &AuthorizationUrl) -> Result<(), String>;
}
#[derive(Default)]
pub struct UnavailableBrowser;
impl BrowserLauncher for UnavailableBrowser {
    fn open(&self, _: &AuthorizationUrl) -> Result<(), String> {
        Err("Browser sign-in is unavailable on this host".into())
    }
}

pub trait HttpClient: Send + Sync {
    fn get<'a>(&'a self, url: &'a str, token: Option<&'a Secret>) -> HttpFuture<'a>;
    fn post_form<'a>(&'a self, url: &'a str, form: Vec<(&'static str, String)>) -> HttpFuture<'a>;
}
pub struct OpenAiHttp(reqwest::Client);
impl Default for OpenAiHttp {
    fn default() -> Self {
        Self(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
        )
    }
}
async fn json_response(
    result: Result<reqwest::Response, reqwest::Error>,
) -> Result<Value, Failure> {
    let mut response = result.map_err(|_| Failure::network())?;
    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::network())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(Failure::local("ChatGPT response exceeded its size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| Failure::local("ChatGPT returned an invalid response"))?
    };
    if (200..300).contains(&status) {
        Ok(value)
    } else {
        Err(Failure::from_response(status, &value, request_id))
    }
}
impl HttpClient for OpenAiHttp {
    fn get<'a>(&'a self, url: &'a str, token: Option<&'a Secret>) -> HttpFuture<'a> {
        Box::pin(async move {
            let mut request = self.0.get(url);
            if let Some(token) = token {
                request = request.bearer_auth(token.expose());
            }
            json_response(request.send().await).await
        })
    }
    fn post_form<'a>(&'a self, url: &'a str, form: Vec<(&'static str, String)>) -> HttpFuture<'a> {
        Box::pin(async move {
            let request = self.0.post(url).form(&form);
            let mut form = form;
            for (_, value) in &mut form {
                value.zeroize();
            }
            json_response(request.send().await).await
        })
    }
}

/// Headless account operations never call a real network service.
struct OfflineHttp;
impl HttpClient for OfflineHttp {
    fn get<'a>(&'a self, _: &'a str, _: Option<&'a Secret>) -> HttpFuture<'a> {
        Box::pin(async { Err(Failure::network()) })
    }
    fn post_form<'a>(&'a self, _: &'a str, _: Vec<(&'static str, String)>) -> HttpFuture<'a> {
        Box::pin(async { Err(Failure::network()) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    #[default]
    SignedOut,
    Ready,
    ConsentRequired,
    ReconnectRequired,
    Paused,
    Restricted,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub state: AccountState,
    pub status: Option<u16>,
    #[serde(default)]
    pub body_shape: ErrorBodyShape,
    pub code: Option<String>,
    pub param: Option<String>,
    pub request_id: Option<String>,
    pub message: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ErrorBodyShape {
    #[default]
    Local,
    OAuth,
    ApiError,
    DirectAdmission,
    OtherObject,
    NonObject,
}
impl Failure {
    #[must_use]
    pub fn local(message: &str) -> Self {
        Self {
            state: AccountState::Unavailable,
            status: None,
            body_shape: ErrorBodyShape::Local,
            code: None,
            param: None,
            request_id: None,
            message: message.into(),
        }
    }
    #[must_use]
    pub fn network() -> Self {
        Self::local("ChatGPT is temporarily unavailable. Try again later.")
    }
    #[must_use]
    pub fn from_response(status: u16, body: &Value, request_id: Option<String>) -> Self {
        // OAuth and inference error bodies are untrusted. Never copy raw detail,
        // token responses, or authorization URLs into a user-visible diagnostic.
        let error = body.get("error").unwrap_or(body);
        let body_shape = match body.get("error") {
            Some(Value::String(_)) => ErrorBodyShape::OAuth,
            Some(Value::Object(_)) => ErrorBodyShape::ApiError,
            _ if body.get("detail").is_some() => ErrorBodyShape::DirectAdmission,
            _ if body.is_object() => ErrorBodyShape::OtherObject,
            _ => ErrorBodyShape::NonObject,
        };
        let code = error
            .as_str()
            .or_else(|| error.get("code").and_then(Value::as_str));
        let code = code
            .filter(|s| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .map(str::to_owned);
        let terminal_refresh = code.as_deref().is_some_and(|c| {
            matches!(
                c,
                "invalid_grant"
                    | "invalid_refresh_token"
                    | "token_expired"
                    | "refresh_token_expired"
                    | "refresh_token_invalidated"
                    | "refresh_token_reused"
            )
        });
        let (state, message) = if terminal_refresh {
            (
                AccountState::ReconnectRequired,
                "Reconnect your ChatGPT account.",
            )
        } else {
            match code.as_deref() {
                Some("subscription_sharing_usage_limit_exceeded") => (
                    AccountState::Paused,
                    "ChatGPT usage limit reached. Manage usage in ChatGPT settings.",
                ),
                Some(
                    "subscription_sharing_invalid_user"
                    | "subscription_sharing_user_not_eligible"
                    | "subscription_sharing_unsupported_capability"
                    | "subscription_sharing_route_not_supported"
                    | "chatpass_v2_scope_not_authorized"
                    | "chatpass_v2_invalid_authorization_context"
                    | "invalid_client",
                ) => (
                    AccountState::Restricted,
                    "ChatGPT plan access is restricted for this account or request.",
                ),
                _ if status == 401 || status == 403 => (
                    AccountState::Restricted,
                    "ChatGPT could not authorize this request. Check the account and plan permission.",
                ),
                _ => (
                    AccountState::Unavailable,
                    "ChatGPT is temporarily unavailable. Try again later.",
                ),
            }
        };
        let param = error
            .get("param")
            .and_then(Value::as_str)
            .filter(|s| {
                s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_./[]".contains(&b))
            })
            .map(str::to_owned);
        Self {
            state,
            status: Some(status),
            body_shape,
            code,
            param,
            request_id: request_id.filter(|s| {
                s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            }),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        if let Some(code) = &self.code {
            write!(f, " ({code})")?;
        }
        if let Some(id) = &self.request_id {
            write!(f, " Request ID: {id}")?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub slug: String,
    pub display_name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    pub id: AccountId,
    pub label: String,
    pub email: String,
    pub state: AccountState,
    #[serde(default)]
    pub generation: u64,
    pub model: Option<String>,
    pub models: Vec<Model>,
    pub failure: Option<Failure>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub accounts: Vec<AccountView>,
    pub active: Option<AccountId>,
    pub busy: bool,
    #[serde(default)]
    pub storage_unavailable: bool,
    pub error: Option<String>,
    pub welcome: bool,
}
impl AccountSnapshot {
    #[must_use]
    pub fn selected(&self) -> Option<&AccountView> {
        self.accounts
            .iter()
            .find(|a| Some(&a.id) == self.active.as_ref())
    }
    /// Connected registrations retain a routing barrier even without usable tokens.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.selected().is_some()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountCommand {
    Refresh,
    Connect {
        account: Option<AccountId>,
        consent: bool,
    },
    Cancel,
    SelectAccount(AccountId),
    SelectModel(String),
    SignOut(AccountId),
    DismissWelcome,
}
#[derive(Debug, Clone)]
pub enum AccountEvent {
    Changed(AccountSnapshot),
    Notice(String),
}
pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}
pub struct ChatGptPorts {
    pub storage: Arc<dyn wfdiag_native_settings::ProtectedDocumentStorage>,
    pub http: Arc<dyn HttpClient>,
    pub browser: Arc<dyn BrowserLauncher>,
    pub clock: Arc<dyn Clock>,
}
impl ChatGptPorts {
    #[must_use]
    pub fn shipping(browser: Arc<dyn BrowserLauncher>) -> Self {
        Self {
            storage: Arc::new(wfdiag_native_settings::WindowsDpapiDocumentStorage),
            http: Arc::new(OpenAiHttp::default()),
            browser,
            clock: Arc::new(SystemClock),
        }
    }
    #[must_use]
    pub fn mock() -> Self {
        Self {
            storage: Arc::new(wfdiag_native_settings::MemoryProtectedDocumentStorage::default()),
            http: Arc::new(OfflineHttp),
            browser: Arc::new(UnavailableBrowser),
            clock: Arc::new(SystemClock),
        }
    }
}
