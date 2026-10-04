use crate::oauth;
use crate::{
    API_BASE, AccountCommand, AccountEvent, AccountId, AccountSnapshot, AccountState, AccountView,
    ChatGptPorts, DISCOVERY_URL, Failure, Model, PLAN_SCOPE, Secret, TOKEN_URL,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wfdiag_native_settings::ProtectedDocumentGuard;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access: Secret,
    refresh: Secret,
    identity: Option<Secret>,
    scopes: Vec<String>,
    expires_at: u64,
    earliest_refresh_at: u64,
}
impl Tokens {
    fn from_response(value: &Value, now: u64, previous: Option<&Self>) -> Result<Self, String> {
        let secret = |name| {
            value
                .get(name)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| Secret::new(s.into()))
        };
        if value
            .get("token_type")
            .and_then(Value::as_str)
            .is_none_or(|s| !s.eq_ignore_ascii_case("Bearer"))
        {
            return Err("ChatGPT returned an unsupported token type".into());
        }
        let expires = value
            .get("expires_in")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or("ChatGPT token expiry was missing")?;
        Ok(Self {
            access: secret("access_token").ok_or("ChatGPT access token was missing")?,
            refresh: secret("refresh_token").ok_or("ChatGPT renewable session was missing")?,
            identity: secret("id_token").or_else(|| previous.and_then(|p| p.identity.clone())),
            scopes: value.get("scope").and_then(Value::as_str).map_or_else(
                || previous.map_or_else(Vec::new, |p| p.scopes.clone()),
                |s| s.split_whitespace().map(str::to_owned).collect(),
            ),
            expires_at: now
                .checked_add(expires)
                .ok_or("Invalid ChatGPT token expiry")?,
            earliest_refresh_at: value
                .get("earliest_refresh_at")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })
    }
    fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Account {
    id: AccountId,
    client_id: String,
    subject: String,
    email: String,
    label: String,
    tokens: Option<Tokens>,
    model: Option<String>,
    models: Vec<Model>,
    failure: Option<Failure>,
    welcome_seen: bool,
    revision: u64,
}
impl Account {
    fn view(&self) -> AccountView {
        let state = self.failure.as_ref().map_or_else(
            || match &self.tokens {
                Some(tokens) if tokens.plan_enabled() => AccountState::Ready,
                Some(_) => AccountState::ConsentRequired,
                None => AccountState::SignedOut,
            },
            |e| e.state,
        );
        AccountView {
            id: self.id.clone(),
            label: self.label.clone(),
            email: self.email.clone(),
            state,
            generation: self.revision,
            model: self.model.clone(),
            models: self.models.clone(),
            failure: self.failure.clone(),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    host_id: String,
    accounts: Vec<Account>,
    active: Option<AccountId>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            accounts: Vec::new(),
            active: None,
        }
    }
}
fn read(guard: &dyn ProtectedDocumentGuard) -> Result<Document, String> {
    let Some(text) = guard.load()? else {
        return Ok(Document::default());
    };
    let document: Document = serde_json::from_str(&text)
        .map_err(|_| "ChatGPT account storage is invalid; reconnect your account")?;
    if document.version != 1
        || document
            .host_id
            .strip_prefix("urn:uuid:")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .is_none_or(|id| id.get_version_num() != 4)
    {
        return Err("Unsupported ChatGPT account storage".into());
    }
    Ok(document)
}
fn save(guard: &dyn ProtectedDocumentGuard, document: &Document) -> Result<(), String> {
    let text = Zeroizing::new(
        serde_json::to_string(document).map_err(|_| "ChatGPT accounts could not be encoded")?,
    );
    guard.save(&text)
}

#[derive(Clone)]
pub struct SessionLease {
    pub account_id: AccountId,
    pub model: String,
    revision: u64,
    pub cancel: CancellationToken,
}
impl std::fmt::Debug for SessionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLease")
            .field("account_id", &self.account_id)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}
impl SessionLease {
    #[must_use]
    pub fn fingerprint(&self) -> String {
        format!(
            "chatgpt:{}:{}:{}",
            self.account_id, self.revision, self.model
        )
    }
}
type Wake = Arc<dyn Fn() + Send + Sync>;
pub struct ChatGptService {
    ports: ChatGptPorts,
    serial: tokio::sync::Mutex<()>,
    busy: AtomicBool,
    renewal_blocked: Mutex<Vec<AccountId>>,
    snapshot: Mutex<AccountSnapshot>,
    events: Mutex<VecDeque<AccountEvent>>,
    wake: Mutex<Option<Wake>>,
    leases: Mutex<Vec<(AccountId, CancellationToken)>>,
}
impl std::fmt::Debug for ChatGptService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatGptService").finish_non_exhaustive()
    }
}
impl ChatGptService {
    #[must_use]
    pub fn new(ports: ChatGptPorts) -> Self {
        Self {
            ports,
            serial: tokio::sync::Mutex::new(()),
            busy: AtomicBool::new(false),
            renewal_blocked: Mutex::new(Vec::new()),
            snapshot: Mutex::new(AccountSnapshot::default()),
            events: Mutex::new(VecDeque::new()),
            wake: Mutex::new(None),
            leases: Mutex::new(Vec::new()),
        }
    }
    #[must_use]
    pub fn snapshot(&self) -> AccountSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn set_wake_handler(&self, wake: Wake) {
        *self
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(wake);
    }
    pub fn take_events(&self) -> Vec<AccountEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
    fn emit(&self, event: AccountEvent) {
        // Only snapshots and bounded notices cross this boundary.
        if let AccountEvent::Changed(snapshot) = &event {
            *self
                .snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot.clone();
        }
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(event, AccountEvent::Changed(_)) {
            events.retain(|e| !matches!(e, AccountEvent::Changed(_)));
        }
        if events.len() < 32 {
            events.push_back(event);
        }
        drop(events);
        if let Some(wake) = self
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            wake();
        }
    }
    fn publish(&self, document: &Document) {
        let mut snapshot = AccountSnapshot {
            accounts: document.accounts.iter().map(Account::view).collect(),
            active: document.active.clone(),
            busy: self.busy.load(Ordering::Acquire),
            ..AccountSnapshot::default()
        };
        snapshot.welcome = document.accounts.iter().any(|a| {
            Some(&a.id) == document.active.as_ref()
                && !a.welcome_seen
                && a.tokens.as_ref().is_some_and(Tokens::plan_enabled)
        });
        self.emit(AccountEvent::Changed(snapshot));
    }
    pub fn cancel_generations(&self) {
        for (_, cancel) in self
            .leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            cancel.cancel();
        }
    }
    async fn locked(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn ProtectedDocumentGuard>, String> {
        let wait = async {
            loop {
                let store = Arc::clone(&self.ports.storage);
                if let Some(guard) = tokio::task::spawn_blocking(move || store.try_lock())
                    .await
                    .map_err(|_| "ChatGPT storage worker stopped")??
                {
                    return Ok(guard);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::select! {
            () = cancel.cancelled() => Err("ChatGPT account action cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(30), wait) => result.map_err(|_| "ChatGPT account storage is busy")?,
        }
    }
    pub async fn execute(
        self: &Arc<Self>,
        command: AccountCommand,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        let storage_probe = matches!(command, AccountCommand::Refresh);
        let signing_out = match &command {
            AccountCommand::SignOut(id) => Some(id.clone()),
            _ => None,
        };
        let interactive = !storage_probe;
        if interactive {
            self.busy.store(true, Ordering::Release);
            let mut snapshot = self.snapshot();
            snapshot.busy = true;
            snapshot.error = None;
            self.emit(AccountEvent::Changed(snapshot));
        }
        let result = tokio::select! {
            () = cancel.cancelled() => Err("ChatGPT account action cancelled".into()),
            result = self.execute_inner(command, &cancel) => result,
        };
        if interactive {
            self.busy.store(false, Ordering::Release);
        }
        if result.is_err()
            && cancel.is_cancelled()
            && signing_out.is_some_and(|id| {
                self.snapshot()
                    .accounts
                    .iter()
                    .any(|a| a.id == id && a.state == AccountState::SignedOut)
            })
        {
            self.revocation_unconfirmed();
        }
        if interactive || result.is_err() {
            let mut snapshot = self.snapshot();
            snapshot.busy = self.busy.load(Ordering::Acquire);
            snapshot.error = result.as_ref().err().cloned();
            if storage_probe {
                snapshot.storage_unavailable = result.is_err();
            }
            self.emit(AccountEvent::Changed(snapshot));
        }
        result
    }
    async fn execute_inner(
        self: &Arc<Self>,
        command: AccountCommand,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if let AccountCommand::Connect { account, consent } = command {
            return self.connect(account, consent, cancel).await;
        }
        let serial = self.serial.lock().await;
        let guard = self.locked(cancel).await?;
        let mut document = read(guard.as_ref())?;
        match command {
            AccountCommand::Refresh => {
                self.publish(&document);
            }
            AccountCommand::SelectAccount(id) => {
                if !document.accounts.iter().any(|a| a.id == id) {
                    return Err("ChatGPT account was not found".into());
                }
                self.cancel_generations();
                document.active = Some(id);
                save(guard.as_ref(), &document)?;
                self.publish(&document);
                let ready = self
                    .snapshot()
                    .selected()
                    .is_some_and(|a| a.state == AccountState::Ready);
                drop(guard);
                drop(serial);
                if ready {
                    self.refresh_models(cancel).await?;
                }
                return Ok(());
            }
            AccountCommand::SelectModel(slug) => {
                let id = document
                    .active
                    .clone()
                    .ok_or("Connect a ChatGPT account first")?;
                let account = document
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("ChatGPT account was not found")?;
                if !account.models.iter().any(|m| m.slug == slug) {
                    return Err("Select a model from this ChatGPT account's catalog".into());
                }
                self.cancel_generations();
                account.model = Some(slug);
                account.revision = account.revision.saturating_add(1);
                save(guard.as_ref(), &document)?;
                self.publish(&document);
            }
            AccountCommand::SignOut(id) => {
                return self.sign_out(id, guard.as_ref(), &mut document).await;
            }
            AccountCommand::DismissWelcome => {
                if let Some(account) = document
                    .accounts
                    .iter_mut()
                    .find(|a| Some(&a.id) == document.active.as_ref())
                {
                    account.welcome_seen = true;
                }
                save(guard.as_ref(), &document)?;
                self.publish(&document);
            }
            AccountCommand::Cancel => {}
            AccountCommand::Connect { .. } => unreachable!(),
        }
        Ok(())
    }
    async fn sign_out(
        &self,
        id: AccountId,
        guard: &dyn ProtectedDocumentGuard,
        document: &mut Document,
    ) -> Result<(), String> {
        self.cancel_generations();
        let account = document
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or("ChatGPT account was not found")?;
        let client_id = account.client_id.clone();
        let tokens = account.tokens.take();
        account.failure = None;
        account.revision = account.revision.saturating_add(1);
        if document.active.as_ref() == Some(&id) {
            document.active = None;
        }
        // Persist local sign-out before any network wait or cancellation point.
        save(guard, document)?;
        self.publish(document);
        // Hold the old token only for bounded revocation, never for inference.
        let mut revoked = tokens.is_none();
        if let Some(tokens) = tokens {
            let discovery = self.ports.http.get(DISCOVERY_URL, None).await;
            if let Ok(discovery) = discovery
                && let Some(endpoint) = discovery
                    .get("revocation_endpoint")
                    .and_then(Value::as_str)
                    .filter(|u| *u == "https://auth.openai.com/api/accounts/oauth/revoke")
            {
                for attempt in 0..3 {
                    let result = self
                        .ports
                        .http
                        .post_form(
                            endpoint,
                            vec![
                                ("token", tokens.refresh.expose().into()),
                                ("token_type_hint", "refresh_token".into()),
                                ("client_id", client_id.clone()),
                            ],
                        )
                        .await;
                    match result {
                        Ok(_) => {
                            revoked = true;
                            break;
                        }
                        Err(e) if e.status.is_none_or(|s| s >= 500) => {
                            tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        save(guard, document)?;
        self.publish(document);
        if !revoked {
            self.revocation_unconfirmed();
        }
        Ok(())
    }
    fn revocation_unconfirmed(&self) {
        self.emit(AccountEvent::Notice("Signed out locally. Remote revocation was not confirmed; disconnect WindowsForum Diagnostics in ChatGPT settings.".into()));
    }
    // Keep browser authorization, registration persistence and token validation in one transaction.
    #[allow(clippy::too_many_lines)]
    async fn connect(
        self: &Arc<Self>,
        account: Option<AccountId>,
        consent: bool,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let (host, previous) = {
            let _serial = self.serial.lock().await;
            let guard = self.locked(cancel).await?;
            let document = read(guard.as_ref())?;
            save(guard.as_ref(), &document)?; // Host identity exists before opening the browser.
            let previous = account
                .as_ref()
                .map(|id| {
                    document
                        .accounts
                        .iter()
                        .find(|a| &a.id == id)
                        .cloned()
                        .ok_or("ChatGPT account was not found")
                })
                .transpose()?;
            (document.host_id, previous)
        };
        let mut snapshot = self.snapshot();
        snapshot.busy = true;
        snapshot.error = None;
        self.emit(AccountEvent::Changed(snapshot));
        let saved = previous.as_ref().map(|a| {
            (
                a.client_id.as_str(),
                a.email.as_str(),
                a.tokens.as_ref().and_then(|t| t.identity.as_ref()),
            )
        });
        let authorization = oauth::authorize(&self.ports, &host, saved, consent, cancel).await?;
        let id = previous
            .as_ref()
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), |a| a.id.clone());
        if previous.is_none() {
            // Keep the issued public registration even if code exchange is interrupted.
            // It contains no unverified identity and cannot run inference.
            let _serial = self.serial.lock().await;
            let guard = self.locked(cancel).await?;
            let mut document = read(guard.as_ref())?;
            document.accounts.push(Account {
                id: id.clone(),
                client_id: authorization.client_id.clone(),
                subject: String::new(),
                email: String::new(),
                label: format!(
                    "Pending ChatGPT connection · account {}",
                    document.accounts.len() + 1
                ),
                tokens: None,
                model: None,
                models: Vec::new(),
                failure: None,
                welcome_seen: false,
                revision: 0,
            });
            save(guard.as_ref(), &document)?;
            self.publish(&document);
        }
        let auth = oauth::exchange(&self.ports, authorization).await?;
        if previous
            .as_ref()
            .is_some_and(|a| !a.subject.is_empty() && a.subject != auth.identity.sub)
        {
            return Err(
                "ChatGPT signed in to a different account than the selected registration".into(),
            );
        }
        let tokens = Tokens::from_response(&auth.tokens, self.ports.clock.now(), None)?;
        let serial = self.serial.lock().await;
        let guard = self.locked(cancel).await?;
        let mut document = read(guard.as_ref())?;
        if previous.as_ref().is_some_and(|previous| {
            !document
                .accounts
                .iter()
                .any(|a| a.id == previous.id && a.revision == previous.revision)
        }) {
            return Err("ChatGPT registration changed during sign-in; try again".into());
        }
        let email = auth.identity.email.unwrap_or_default();
        let account_number = document
            .accounts
            .iter()
            .position(|a| a.id == id)
            .map_or(document.accounts.len() + 1, |index| index + 1);
        let mut account = Account {
            id: id.clone(),
            client_id: auth.client_id,
            subject: auth.identity.sub,
            label: format!(
                "{} · account {}",
                if email.is_empty() { "ChatGPT" } else { &email },
                account_number
            ),
            email,
            tokens: Some(tokens),
            model: None,
            models: Vec::new(),
            failure: None,
            welcome_seen: previous.as_ref().is_some_and(|a| a.welcome_seen),
            revision: previous
                .as_ref()
                .map_or(1, |a| a.revision.saturating_add(1)),
        };
        if let Some(index) = document.accounts.iter().position(|a| a.id == id) {
            if !document.accounts[index].subject.is_empty() {
                account.label.clone_from(&document.accounts[index].label);
            }
            document.accounts[index] = account;
        } else {
            document.accounts.push(account);
        }
        self.cancel_generations();
        self.renewal_blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|a| a != &id);
        document.active = Some(id);
        save(guard.as_ref(), &document)?;
        self.publish(&document);
        drop(guard);
        drop(serial);
        if self
            .snapshot()
            .selected()
            .is_some_and(|a| a.state == AccountState::Ready)
        {
            self.refresh_models(cancel).await?;
        }
        Ok(())
    }
    pub async fn lease(
        self: &Arc<Self>,
        cancel: &CancellationToken,
    ) -> Result<SessionLease, String> {
        if self.snapshot().accounts.is_empty() {
            self.execute(AccountCommand::Refresh, cancel.clone())
                .await?;
        }
        let snapshot = self.snapshot();
        let account = snapshot.selected().ok_or_else(|| {
            snapshot
                .error
                .clone()
                .unwrap_or_else(|| "Continue with ChatGPT in Settings to connect your plan".into())
        })?;
        if matches!(
            account.state,
            AccountState::ConsentRequired
                | AccountState::ReconnectRequired
                | AccountState::Paused
                | AccountState::Restricted
                | AccountState::SignedOut
        ) {
            return Err(account.failure.as_ref().map_or_else(
                || "Enable ChatGPT plan usage or reconnect your account in Settings".into(),
                ToString::to_string,
            ));
        }
        if account.model.is_none() {
            self.refresh_models(cancel).await?;
        }
        let _serial = self.serial.lock().await;
        let guard = self.locked(cancel).await?;
        let document = read(guard.as_ref())?;
        let account = document
            .accounts
            .iter()
            .find(|a| a.id == account.id && Some(&a.id) == document.active.as_ref())
            .ok_or("ChatGPT account changed during preparation")?;
        if !matches!(
            account.view().state,
            AccountState::Ready | AccountState::Unavailable
        ) || account.tokens.as_ref().is_none_or(|t| !t.plan_enabled())
        {
            return Err("ChatGPT account needs attention in Settings".into());
        }
        if self
            .renewal_blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&account.id)
        {
            return Err("ChatGPT renewal could not be saved. Reconnect your account.".into());
        }
        let lease = SessionLease {
            account_id: account.id.clone(),
            revision: account.revision,
            model: account
                .model
                .clone()
                .ok_or("This ChatGPT account has no available models")?,
            cancel: CancellationToken::new(),
        };
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        leases.retain(|(_, c)| !c.is_cancelled());
        if leases.len() >= 128 {
            leases.remove(0).1.cancel();
        }
        leases.push((lease.account_id.clone(), lease.cancel.clone()));
        Ok(lease)
    }
    /// Revalidate account ownership before publishing a completed response or running tools.
    pub async fn validate_lease(&self, lease: &SessionLease) -> Result<(), String> {
        if lease.cancel.is_cancelled() {
            return Err("ChatGPT account changed; start a new request".into());
        }
        let _serial = self.serial.lock().await;
        let guard = self.locked(&lease.cancel).await?;
        let document = read(guard.as_ref())?;
        if !document.accounts.iter().any(|a| {
            a.id == lease.account_id
                && a.revision == lease.revision
                && Some(&a.id) == document.active.as_ref()
                && a.model.as_deref() == Some(lease.model.as_str())
                && a.tokens.as_ref().is_some_and(Tokens::plan_enabled)
        }) {
            return Err("ChatGPT account changed; start a new request".into());
        }
        Ok(())
    }
    /// Complete renewal and atomic persistence even if the generation is cancelled.
    pub async fn access_token(self: &Arc<Self>, lease: &SessionLease) -> Result<Secret, String> {
        let service = Arc::clone(self);
        let lease = lease.clone();
        tokio::spawn(async move { service.access_token_inner(&lease).await })
            .await
            .map_err(|_| "ChatGPT renewal worker stopped")?
    }
    // Rotation and its failure handling form one locked persistence transaction.
    #[allow(clippy::too_many_lines)]
    async fn access_token_inner(&self, lease: &SessionLease) -> Result<Secret, String> {
        if self
            .renewal_blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&lease.account_id)
        {
            return Err("ChatGPT renewal could not be saved. Reconnect your account.".into());
        }
        let _serial = self.serial.lock().await;
        let guard = self.locked(&lease.cancel).await?;
        let mut document = read(guard.as_ref())?;
        let active = document.active.clone();
        let account = document
            .accounts
            .iter_mut()
            .find(|a| {
                a.id == lease.account_id
                    && a.revision == lease.revision
                    && Some(&a.id) == active.as_ref()
                    && (lease.model.is_empty() || a.model.as_deref() == Some(lease.model.as_str()))
            })
            .ok_or("ChatGPT session changed; start a new request")?;
        if let Some(failure) = &account.failure
            && matches!(
                failure.state,
                AccountState::Paused | AccountState::Restricted | AccountState::ReconnectRequired
            )
        {
            return Err(failure.to_string());
        }
        let tokens = account
            .tokens
            .as_ref()
            .ok_or("Reconnect your ChatGPT account")?;
        if !tokens.plan_enabled() {
            return Err("Enable ChatGPT plan usage in Settings".into());
        }
        let now = self.ports.clock.now();
        if tokens.expires_at > now.saturating_add(60) {
            return Ok(tokens.access.clone());
        }
        if tokens.earliest_refresh_at > now {
            return Err("ChatGPT token renewal is not available yet. Try again later.".into());
        }
        let refreshed = self
            .ports
            .http
            .post_form(
                TOKEN_URL,
                vec![
                    ("grant_type", "refresh_token".into()),
                    ("client_id", account.client_id.clone()),
                    ("refresh_token", tokens.refresh.expose().into()),
                    ("resource", API_BASE.into()),
                ],
            )
            .await;
        match refreshed {
            Ok(response) => {
                let replacement = Tokens::from_response(&response, now, Some(tokens))?;
                let access = replacement.access.clone();
                account.tokens = Some(replacement);
                account.failure = None;
                // Never send inference or reuse the obsolete refresh token when persistence failed.
                if let Err(error) = save(guard.as_ref(), &document) {
                    self.renewal_blocked
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(lease.account_id.clone());
                    self.cancel_generations();
                    let mut failure = Failure::local(
                        "ChatGPT renewal could not be saved. Reconnect your account.",
                    );
                    failure.state = AccountState::ReconnectRequired;
                    if let Some(account) = document
                        .accounts
                        .iter_mut()
                        .find(|a| a.id == lease.account_id)
                    {
                        account.failure = Some(failure);
                    }
                    self.publish(&document);
                    return Err(error);
                }
                self.publish(&document);
                Ok(access)
            }
            Err(failure) => {
                if failure.state == AccountState::ReconnectRequired {
                    account.tokens = None;
                }
                account.failure = Some(failure.clone());
                if let Err(error) = save(guard.as_ref(), &document) {
                    if failure.state == AccountState::ReconnectRequired {
                        self.renewal_blocked
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(lease.account_id.clone());
                    }
                    self.publish(&document);
                    return Err(error);
                }
                self.publish(&document);
                Err(failure.to_string())
            }
        }
    }
    pub async fn record_failure(&self, lease: &SessionLease, failure: Failure) {
        let _serial = self.serial.lock().await;
        if let Ok(guard) = self.locked(&lease.cancel).await
            && let Ok(mut document) = read(guard.as_ref())
            && let Some(account) = document
                .accounts
                .iter_mut()
                .find(|a| a.id == lease.account_id && a.revision == lease.revision)
        {
            // A transport wrapper must not weaken an already recorded renewal/quota failure.
            if account.failure.as_ref().is_some_and(|f| {
                matches!(
                    f.state,
                    AccountState::Paused
                        | AccountState::Restricted
                        | AccountState::ReconnectRequired
                )
            }) && failure.state == AccountState::Unavailable
            {
                return;
            }
            account.failure = Some(failure);
            let _ = save(guard.as_ref(), &document);
            self.publish(&document);
        }
    }
    pub async fn refresh_models(
        self: &Arc<Self>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Model>, String> {
        // Temporary lease uses the persisted revision; catalog and inference share renewal.
        let (id, revision) = {
            let _serial = self.serial.lock().await;
            let guard = self.locked(cancel).await?;
            let document = read(guard.as_ref())?;
            let account = document
                .accounts
                .iter()
                .find(|a| Some(&a.id) == document.active.as_ref())
                .ok_or("Connect a ChatGPT account first")?;
            (account.id.clone(), account.revision)
        };
        let lease = SessionLease {
            account_id: id,
            revision,
            model: String::new(),
            cancel: cancel.clone(),
        };
        let token = self.access_token(&lease).await?;
        let response = self
            .ports
            .http
            .get("https://api.openai.com/v1/models", Some(&token))
            .await;
        let response = match response {
            Ok(value) => value,
            Err(failure) => {
                self.record_failure(&lease, failure.clone()).await;
                return Err(failure.to_string());
            }
        };
        let models: Vec<Model> = response
            .get("models")
            .and_then(Value::as_array)
            .ok_or("Invalid ChatGPT model catalog")?
            .iter()
            .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
            .filter_map(|m| {
                Some(Model {
                    slug: m.get("slug")?.as_str()?.into(),
                    display_name: m.get("display_name")?.as_str()?.into(),
                })
            })
            .collect();
        let _serial = self.serial.lock().await;
        let guard = self.locked(cancel).await?;
        let mut document = read(guard.as_ref())?;
        let active = document.active.clone();
        let account = document
            .accounts
            .iter_mut()
            .find(|a| {
                a.id == lease.account_id && a.revision == revision && Some(&a.id) == active.as_ref()
            })
            .ok_or("ChatGPT account changed during model discovery")?;
        let previous_model = account.model.clone();
        if !models
            .iter()
            .any(|m| account.model.as_ref() == Some(&m.slug))
        {
            account.model = models.first().map(|m| m.slug.clone());
        }
        if account.model != previous_model {
            account.revision = account.revision.saturating_add(1);
            self.cancel_generations();
        }
        account.models.clone_from(&models);
        if account
            .failure
            .as_ref()
            .is_some_and(|f| f.state == AccountState::Unavailable)
        {
            account.failure = None;
        }
        save(guard.as_ref(), &document)?;
        self.publish(&document);
        Ok(models)
    }
}

#[cfg(test)]
mod tests;
