//! Account-aware model catalogs reported by the genuine subscription CLIs.
//!
//! Listing is read-only. It never invokes a login/logout/install command and
//! never reads or returns vendor credentials. Codex speaks its documented
//! app-server JSONL protocol; Claude reads the model selector from an ACP
//! session initialized without sending a prompt.

use crate::{acp_bridge, cli_bridge};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use wfdiag_native_ai_provider::{
    BackendFuture, ModelCatalog, ModelCatalogEntry, SubscriptionCli, SubscriptionModelCatalogSource,
};

const CODEX_MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(60);
const MODEL_LIST_MAX_PAGES: usize = 50;
const MODEL_LIST_STDOUT_LIMIT: u64 = 2 * 1024 * 1024;
const MODEL_LIST_STDERR_LIMIT: u64 = 32 * 1024;
const CATALOG_TTL: Duration = Duration::from_secs(60);

type CatalogCache = Arc<Mutex<HashMap<String, (Instant, ModelCatalog)>>>;

fn shared_cache() -> CatalogCache {
    static CACHE: OnceLock<CatalogCache> = OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Concrete model source shared by Tauri and Reactor.
///
/// Successful catalogs are cached for 60 seconds by provider plus the freshly
/// resolved executable path; failures are never cached. Each request validates
/// that the CLI still resolves before consulting the cache. Constructing this
/// type has no process or authentication side effects.
#[derive(Clone)]
pub struct ProcessSubscriptionModelCatalogSource {
    cache: CatalogCache,
    /// Per-key async mutexes: concurrent cache misses share one listing
    /// (a real CLI subprocess round trip) instead of each caller spawning
    /// its own. Entries live for the process lifetime but are bounded by the
    /// key space (2 providers x the few resolved paths a user can have).
    in_flight: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl std::fmt::Debug for ProcessSubscriptionModelCatalogSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessSubscriptionModelCatalogSource")
            .finish_non_exhaustive()
    }
}

impl Default for ProcessSubscriptionModelCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSubscriptionModelCatalogSource {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: shared_cache(),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Invalidate account-aware model data after an explicit auth mutation.
    pub fn invalidate(&self, provider: SubscriptionCli) {
        let prefix = format!("{}|", provider_key(provider));
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|key, _| !key.starts_with(&prefix));
        }
    }

    fn cached_catalog(&self, cache_key: &str) -> Option<ModelCatalog> {
        self.cache.lock().ok().and_then(|cache| {
            cache
                .get(cache_key)
                .filter(|(at, _)| at.elapsed() < CATALOG_TTL)
                .map(|(_, catalog)| catalog.clone())
        })
    }

    async fn list_inner(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> Result<ModelCatalog, String> {
        let configured_path = configured_path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
        // Resolution deliberately precedes the cache lookup. Otherwise an
        // executable removed (or a now-invalid explicit path) could keep
        // returning a successful account catalog for the entire TTL.
        let binary = match provider {
            SubscriptionCli::Codex => "codex",
            SubscriptionCli::ClaudeCode => "claude",
        };
        let resolved_path = cli_bridge::resolve_cli(binary, configured_path.as_deref()).await?;
        let cache_key = catalog_cache_key(provider, &resolved_path);
        if let Some(catalog) = self.cached_catalog(&cache_key) {
            return Ok(catalog);
        }

        // Single-flight: serialize concurrent misses per key, then re-check
        // the cache so followers adopt the leader's fresh result instead of
        // each spawning their own CLI subprocess round trip.
        let guard = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            in_flight.entry(cache_key.clone()).or_default().clone()
        };
        let _permit = guard.lock().await;
        if let Some(catalog) = self.cached_catalog(&cache_key) {
            return Ok(catalog);
        }

        let catalog = match provider {
            SubscriptionCli::Codex => list_codex_models(resolved_path).await?,
            SubscriptionCli::ClaudeCode => list_claude_models(&resolved_path).await?,
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(cache_key, (Instant::now(), catalog.clone()));
        }
        Ok(catalog)
    }
}

impl SubscriptionModelCatalogSource for ProcessSubscriptionModelCatalogSource {
    fn list_models(
        &self,
        provider: SubscriptionCli,
        configured_path: Option<String>,
    ) -> BackendFuture<'_, Result<ModelCatalog, String>> {
        Box::pin(async move { self.list_inner(provider, configured_path).await })
    }
}

const fn provider_key(provider: SubscriptionCli) -> &'static str {
    match provider {
        SubscriptionCli::Codex => "codex",
        SubscriptionCli::ClaudeCode => "claude",
    }
}

fn catalog_cache_key(provider: SubscriptionCli, resolved_path: &Path) -> String {
    format!("{}|{}", provider_key(provider), resolved_path.display())
}

async fn list_claude_models(claude: &Path) -> Result<ModelCatalog, String> {
    acp_bridge::list_claude_models(claude)
        .await
        .map(catalog_from_bridge)
        .map_err(|error| safe_claude_catalog_error(&error))
}

fn safe_claude_catalog_error(error: &str) -> String {
    if error.contains("npx was not found") {
        "Claude Code model discovery requires Node.js/npm (npx was not found).".to_string()
    } else if error.contains("did not finish within") || error.contains("timed out") {
        "Claude Code model discovery timed out.".to_string()
    } else if error.contains("did not report a model selector") {
        "Claude Code did not report a model selector.".to_string()
    } else if error.contains("empty model selector") {
        "Claude Code reported an empty model selector.".to_string()
    } else {
        // The ACP adapter's raw stderr can contain arbitrary vendor output.
        // Keep that bounded detail out of the public UI contract.
        "Claude Code model discovery failed. Verify that Claude Code and Node.js/npm are installed and signed in."
            .to_string()
    }
}

fn catalog_from_bridge(catalog: cli_bridge::BridgeModelCatalog) -> ModelCatalog {
    ModelCatalog {
        models: catalog
            .models
            .into_iter()
            .map(|model| ModelCatalogEntry {
                id: model.id,
                label: model.label,
                description: model.description,
                metadata: None,
            })
            .collect(),
        default_model: catalog.default_model,
    }
}

async fn list_codex_models(path: PathBuf) -> Result<ModelCatalog, String> {
    tokio::time::timeout(CODEX_MODEL_LIST_TIMEOUT, list_codex_models_inner(path))
        .await
        .map_err(|_| {
            format!(
                "Codex model discovery did not finish within {} seconds",
                CODEX_MODEL_LIST_TIMEOUT.as_secs()
            )
        })?
}

async fn list_codex_models_inner(path: PathBuf) -> Result<ModelCatalog, String> {
    let workdir = cli_bridge::bridge_workdir().await?;
    let mut command = tokio::process::Command::new(path);
    command.args(["app-server", "--stdio"]);
    command.current_dir(workdir);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    for variable in cli_bridge::SUBSCRIPTION_OVERRIDE_ENV_VARS {
        command.env_remove(variable);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start Codex app-server: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdout".to_string())?
        .take(MODEL_LIST_STDOUT_LIMIT);
    let mut stdout = tokio::io::BufReader::new(stdout);
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut discarded = Vec::new();
            let _ = stderr
                .take(MODEL_LIST_STDERR_LIMIT)
                .read_to_end(&mut discarded)
                .await;
        })
    });

    let result = async {
        send_json_line(
            &mut stdin,
            &json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":{
                    "clientInfo":{
                        "name":"wfdiag",
                        "version":env!("CARGO_PKG_VERSION")
                    },
                    "capabilities":{"experimentalApi":true}
                }
            }),
        )
        .await?;
        let _ = read_jsonrpc_result(&mut stdout, 1).await?;

        let mut catalog = ModelCatalog::default();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for page_index in 0..MODEL_LIST_MAX_PAGES {
            let request_id = page_index as u64 + 2;
            send_json_line(
                &mut stdin,
                &json!({
                    "jsonrpc":"2.0",
                    "id":request_id,
                    "method":"model/list",
                    "params":{
                        "cursor":cursor,
                        "limit":100,
                        "includeHidden":false
                    }
                }),
            )
            .await?;
            let response = read_jsonrpc_result(&mut stdout, request_id).await?;
            let page = parse_codex_model_page(&response)?;
            if catalog.default_model.is_none() {
                catalog.default_model = page.default_model;
            }
            catalog.models.extend(page.models);

            let Some(next) = page.next_cursor else {
                catalog.models = stable_dedupe_entries(catalog.models);
                return Ok(catalog);
            };
            if !seen_cursors.insert(next.clone()) {
                return Err("Codex app-server repeated a model-list cursor".to_string());
            }
            cursor = Some(next);
        }
        Err("Codex app-server model list exceeded the pagination limit".to_string())
    }
    .await;

    drop(stdin);
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    if let Some(task) = stderr_task {
        let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
    }
    result
}

async fn send_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("Could not encode CLI model request: {error}"))?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .map_err(|error| format!("Could not send CLI model request: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("Could not flush CLI model request: {error}"))
}

async fn read_jsonrpc_result<R>(reader: &mut R, wanted_id: u64) -> Result<Value, String>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let mut line = String::new();
        let count = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("Could not read CLI model response: {error}"))?;
        if count == 0 {
            return Err(format!(
                "CLI model server closed before response {wanted_id}"
            ));
        }
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|error| format!("CLI model server returned malformed JSON: {error}"))?;
        if value.get("id").and_then(Value::as_u64) != Some(wanted_id) {
            continue;
        }
        if value.get("error").is_some() {
            return Err("Codex app-server rejected the model-list request".to_string());
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| "CLI model server returned no result".to_string());
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CodexModelPage {
    models: Vec<ModelCatalogEntry>,
    default_model: Option<String>,
    next_cursor: Option<String>,
}

fn parse_codex_model_page(result: &Value) -> Result<CodexModelPage, String> {
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex model list returned no data array".to_string())?;
    let mut page = CodexModelPage {
        next_cursor: result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string),
        ..CodexModelPage::default()
    };
    for raw in data {
        let Some(id) = raw
            .get("model")
            .or_else(|| raw.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if raw.get("isDefault").and_then(Value::as_bool) == Some(true)
            && page.default_model.is_none()
        {
            page.default_model = Some(id.clone());
        }
        let retiring = raw
            .pointer("/upgradeInfo/retirementAt")
            .and_then(Value::as_i64)
            .filter(|at| *at > 0);
        let mut description = non_empty_owned(raw.get("description").and_then(Value::as_str));
        if let Some(at) = retiring {
            let note = format!("Retiring {} — pick a newer model", unix_date(at));
            description = Some(match description {
                Some(existing) => format!("{existing} — {note}"),
                None => note,
            });
        }
        page.models.push(ModelCatalogEntry {
            label: optional_distinct(raw.get("displayName").and_then(Value::as_str), &id),
            description,
            id,
            metadata: None,
        });
    }
    Ok(page)
}

fn optional_distinct(value: Option<&str>, id: &str) -> Option<String> {
    non_empty_owned(value).filter(|value| value != id)
}

fn non_empty_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn stable_dedupe_entries(models: Vec<ModelCatalogEntry>) -> Vec<ModelCatalogEntry> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
}

fn unix_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_page_preserves_live_metadata_and_retirement_notice() {
        let page = parse_codex_model_page(&serde_json::json!({
            "data":[
                {
                    "model":"gpt-5.6-sol",
                    "displayName":"GPT 5.6 Sol",
                    "description":"Frontier coding",
                    "isDefault":true
                },
                {
                    "id":"gpt-5.5-codex",
                    "upgradeInfo":{"retirementAt":1_788_202_800_i64}
                }
            ],
            "nextCursor":"page-2"
        }))
        .unwrap();
        assert_eq!(page.default_model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
        assert_eq!(page.models[0].label.as_deref(), Some("GPT 5.6 Sol"));
        assert_eq!(
            page.models[1].description.as_deref(),
            Some("Retiring 2026-08-31 — pick a newer model")
        );
    }

    #[test]
    fn codex_page_rejects_a_missing_data_array() {
        assert!(parse_codex_model_page(&serde_json::json!({"error":"nope"})).is_err());
    }

    #[test]
    fn public_claude_errors_never_forward_arbitrary_adapter_output() {
        let raw = "Claude Code model discovery failed: bearer sk-ant-secret stderr";
        let safe = safe_claude_catalog_error(raw);
        assert!(!safe.contains("sk-ant-secret"));
        assert!(!safe.contains("stderr"));
    }

    #[test]
    fn account_cache_key_includes_the_explicit_path() {
        assert_ne!(
            catalog_cache_key(SubscriptionCli::Codex, Path::new(r"C:\Tools\codex.exe")),
            catalog_cache_key(SubscriptionCli::Codex, Path::new(r"D:\Tools\codex.exe"))
        );
    }
}
