//! Does a subscription CLI have a stored login? Answered from the *presence*
//! of the vendor's credential cache — `symlink_metadata` existence and size
//! only. The file itself is never opened, so no token can ever be read.
//!
//! Absence is decisive (the CLI cannot be signed in without its cache), so
//! the probe reports "no stored login" without spawning the CLI. Presence is
//! not decisive (the cache may be expired), so it still runs the vendor's
//! status command. Anything that moves or replaces the cache — a vendor
//! config-directory override, Codex's keyring store, Claude's key helper or
//! cloud back ends, the macOS Keychain — makes the answer `Unknown`, which
//! also runs the status command. The shortcut can therefore only ever save a
//! spawn, never invent a state.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Which vendor cache to look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStoreKind {
    /// `~/.codex/auth.json`.
    CodexAuthJson,
    /// `~/.claude/.credentials.json` (Windows and Linux; macOS uses Keychain).
    ClaudeCredentialsJson,
}

/// The finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStore {
    Present,
    Absent,
    Unknown,
}

/// What `symlink_metadata` said about the cache path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFact {
    Missing,
    Empty,
    NonEmpty,
    Symlink,
    Unreadable,
}

/// Environment variables whose mere presence means the vendor's cache is not
/// where the default lookup expects it (or is not a file at all). Presence
/// only disables the shortcut; the values are never read.
pub const VENDOR_CONFIG_ENV_VARS: &[&str] = &[
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// The variables that matter for one kind.
#[must_use]
pub const fn vendor_config_env_vars(kind: CredentialStoreKind) -> &'static [&'static str] {
    match kind {
        CredentialStoreKind::CodexAuthJson => &["CODEX_HOME"],
        CredentialStoreKind::ClaudeCredentialsJson => &[
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ],
    }
}

/// Where the vendor writes its login cache under `home`.
#[must_use]
pub fn credential_store_file(kind: CredentialStoreKind, home: &Path) -> PathBuf {
    match kind {
        CredentialStoreKind::CodexAuthJson => home.join(".codex").join("auth.json"),
        CredentialStoreKind::ClaudeCredentialsJson => {
            home.join(".claude").join(".credentials.json")
        }
    }
}

/// The vendor's non-secret config file that can redirect the store.
#[must_use]
pub fn vendor_config_file(kind: CredentialStoreKind, home: &Path) -> PathBuf {
    match kind {
        CredentialStoreKind::CodexAuthJson => home.join(".codex").join("config.toml"),
        CredentialStoreKind::ClaudeCredentialsJson => home.join(".claude").join("settings.json"),
    }
}

/// Existence and size decide; a link or an unreadable entry decides nothing.
#[must_use]
pub const fn classify_file(fact: FileFact) -> CredentialStore {
    match fact {
        FileFact::Missing | FileFact::Empty => CredentialStore::Absent,
        FileFact::NonEmpty => CredentialStore::Present,
        FileFact::Symlink | FileFact::Unreadable => CredentialStore::Unknown,
    }
}

/// Codex keeps its login in the OS keyring when `config.toml` says so
/// (`cli_auth_credentials_store = "keyring"` or `"auto"`); `auth.json` is
/// then absent even when signed in.
#[must_use]
pub fn codex_config_uses_keyring(config_toml: &str) -> bool {
    config_toml.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        key.trim() == "cli_auth_credentials_store"
            && matches!(
                value.trim().trim_matches(|c| c == '"' || c == '\''),
                "keyring" | "auto"
            )
    })
}

/// Claude Code can take its key from an `apiKeyHelper` command, in which
/// case no credential file exists while it is fully usable.
#[must_use]
pub fn claude_settings_use_key_helper(settings_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(settings_json)
        .ok()
        .and_then(|value| {
            value
                .get("apiKeyHelper")
                .and_then(serde_json::Value::as_str)
                .map(|helper| !helper.trim().is_empty())
        })
        .unwrap_or(false)
}

/// Everything the decision reads, gathered by the host so the decision
/// itself is pure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialEnv {
    /// The user's home directory, when known.
    pub home: Option<PathBuf>,
    /// One of [`vendor_config_env_vars`] is set.
    pub vendor_override_present: bool,
    /// Codex's `config.toml`, when it exists (bounded read).
    pub codex_config_toml: Option<String>,
    /// Claude's `settings.json`, when it exists (bounded read).
    pub claude_settings_json: Option<String>,
}

/// Decide, given the environment and a way to inspect the cache path.
pub fn detect_credential_store(
    kind: CredentialStoreKind,
    env: &CredentialEnv,
    fact: impl Fn(&Path) -> FileFact,
) -> CredentialStore {
    let Some(home) = env.home.as_deref() else {
        return CredentialStore::Unknown;
    };
    if env.vendor_override_present {
        return CredentialStore::Unknown;
    }
    match kind {
        CredentialStoreKind::CodexAuthJson => {
            if env
                .codex_config_toml
                .as_deref()
                .is_some_and(codex_config_uses_keyring)
            {
                return CredentialStore::Unknown;
            }
        }
        CredentialStoreKind::ClaudeCredentialsJson => {
            if cfg!(target_os = "macos") {
                return CredentialStore::Unknown;
            }
            if env
                .claude_settings_json
                .as_deref()
                .is_some_and(claude_settings_use_key_helper)
            {
                return CredentialStore::Unknown;
            }
        }
    }
    classify_file(fact(&credential_store_file(kind, home)))
}

/// `symlink_metadata` on the path: never follows a link, never opens the file.
#[must_use]
pub fn file_fact(path: &Path) -> FileFact {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => FileFact::Symlink,
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => FileFact::NonEmpty,
        Ok(metadata) if metadata.is_file() => FileFact::Empty,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileFact::Missing,
        // A directory or a device at the path, or an unreadable entry.
        Ok(_) | Err(_) => FileFact::Unreadable,
    }
}

/// At most this much of a vendor config file is read.
pub const VENDOR_CONFIG_READ_LIMIT: u64 = 64 * 1024;

fn read_bounded(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(VENDOR_CONFIG_READ_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The environment of the running process. `home` comes from the platform's
/// profile directory; the vendor variables are checked for presence only —
/// a path lookup in the `%LOCALAPPDATA%` sense, not a behaviour knob.
#[must_use]
pub fn host_credential_env(kind: CredentialStoreKind) -> CredentialEnv {
    let home = dirs::home_dir();
    let vendor_override_present = vendor_config_env_vars(kind)
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    let config = home
        .as_deref()
        .map(|home| vendor_config_file(kind, home))
        .and_then(|path| read_bounded(&path));
    CredentialEnv {
        home,
        vendor_override_present,
        codex_config_toml: match kind {
            CredentialStoreKind::CodexAuthJson => config.clone(),
            CredentialStoreKind::ClaudeCredentialsJson => None,
        },
        claude_settings_json: match kind {
            CredentialStoreKind::ClaudeCredentialsJson => config,
            CredentialStoreKind::CodexAuthJson => None,
        },
    }
}

/// How the probe asks about the cache; a test injects a fixed answer.
pub trait CredentialStoreProbe: Send + Sync + 'static {
    fn detect(&self, kind: CredentialStoreKind) -> CredentialStore;
}

/// The running host's answer.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostCredentialStoreProbe;

impl CredentialStoreProbe for HostCredentialStoreProbe {
    fn detect(&self, kind: CredentialStoreKind) -> CredentialStore {
        detect_credential_store(kind, &host_credential_env(kind), file_fact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "wfdiag-credential-store-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn env(home: &Path) -> CredentialEnv {
        CredentialEnv {
            home: Some(home.to_path_buf()),
            ..CredentialEnv::default()
        }
    }

    #[test]
    fn codex_store_path_and_claude_store_path_are_under_home() {
        let home = Path::new("/home/mike");
        assert_eq!(
            credential_store_file(CredentialStoreKind::CodexAuthJson, home),
            Path::new("/home/mike/.codex/auth.json")
        );
        assert_eq!(
            credential_store_file(CredentialStoreKind::ClaudeCredentialsJson, home),
            Path::new("/home/mike/.claude/.credentials.json")
        );
        assert_eq!(
            vendor_config_file(CredentialStoreKind::CodexAuthJson, home),
            Path::new("/home/mike/.codex/config.toml")
        );
    }

    #[test]
    fn missing_or_empty_store_is_absent_and_non_empty_is_present() {
        let home = scratch("presence");
        let kind = CredentialStoreKind::CodexAuthJson;
        assert_eq!(
            detect_credential_store(kind, &env(&home), file_fact),
            CredentialStore::Absent,
            "missing"
        );
        let store = credential_store_file(kind, &home);
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(&store, b"").unwrap();
        assert_eq!(
            detect_credential_store(kind, &env(&home), file_fact),
            CredentialStore::Absent,
            "empty"
        );
        std::fs::write(&store, b"{\"tokens\":{}}").unwrap();
        assert_eq!(
            detect_credential_store(kind, &env(&home), file_fact),
            CredentialStore::Present
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_or_unreadable_store_is_unknown_and_never_followed() {
        let home = scratch("symlink");
        let kind = CredentialStoreKind::ClaudeCredentialsJson;
        let store = credential_store_file(kind, &home);
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/nonexistent-target", &store).unwrap();
        assert_eq!(file_fact(&store), FileFact::Symlink);
        assert_eq!(
            detect_credential_store(kind, &env(&home), file_fact),
            CredentialStore::Unknown
        );
        std::fs::remove_file(&store).unwrap();
        std::fs::create_dir(&store).unwrap();
        assert_eq!(file_fact(&store), FileFact::Unreadable);
        assert_eq!(
            classify_file(FileFact::Unreadable),
            CredentialStore::Unknown
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_keyring_or_auto_mode_disables_the_fast_path() {
        assert!(codex_config_uses_keyring(
            "model = \"gpt-5\"\ncli_auth_credentials_store = \"keyring\"\n"
        ));
        assert!(codex_config_uses_keyring(
            "cli_auth_credentials_store = 'auto'"
        ));
        assert!(!codex_config_uses_keyring(
            "cli_auth_credentials_store = \"file\""
        ));
        assert!(!codex_config_uses_keyring(
            "# cli_auth_credentials_store = \"keyring\""
        ));
        let home = scratch("keyring");
        let mut with_keyring = env(&home);
        with_keyring.codex_config_toml = Some("cli_auth_credentials_store = \"keyring\"".into());
        assert_eq!(
            detect_credential_store(CredentialStoreKind::CodexAuthJson, &with_keyring, file_fact),
            CredentialStore::Unknown
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_key_helper_disables_the_fast_path() {
        assert!(claude_settings_use_key_helper(
            r#"{"apiKeyHelper": "/usr/bin/get-key"}"#
        ));
        assert!(!claude_settings_use_key_helper(r#"{"apiKeyHelper": ""}"#));
        assert!(!claude_settings_use_key_helper(r#"{"model": "opus"}"#));
        assert!(!claude_settings_use_key_helper("not json"));
        let home = scratch("helper");
        let mut with_helper = env(&home);
        with_helper.claude_settings_json = Some(r#"{"apiKeyHelper": "x"}"#.into());
        assert_eq!(
            detect_credential_store(
                CredentialStoreKind::ClaudeCredentialsJson,
                &with_helper,
                file_fact
            ),
            CredentialStore::Unknown
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn vendor_config_dir_override_and_missing_home_disable_the_fast_path() {
        let home = scratch("override");
        let mut overridden = env(&home);
        overridden.vendor_override_present = true;
        assert_eq!(
            detect_credential_store(CredentialStoreKind::CodexAuthJson, &overridden, file_fact),
            CredentialStore::Unknown
        );
        assert_eq!(
            detect_credential_store(
                CredentialStoreKind::CodexAuthJson,
                &CredentialEnv::default(),
                |_| FileFact::Missing
            ),
            CredentialStore::Unknown
        );
        assert_eq!(
            vendor_config_env_vars(CredentialStoreKind::CodexAuthJson),
            ["CODEX_HOME"]
        );
        for name in vendor_config_env_vars(CredentialStoreKind::ClaudeCredentialsJson) {
            assert!(VENDOR_CONFIG_ENV_VARS.contains(name));
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
