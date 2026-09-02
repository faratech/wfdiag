//! The one description of each subscription CLI: its binary, the vendor's
//! status / sign-in / sign-out verbs, the phrases that mean "signed out",
//! the installer identity and where it keeps its login. Every probe, the
//! account controller and the installer read this table; nothing else may
//! carry its own copy.

use crate::composition::SubscriptionCli;
use crate::credential_store::CredentialStoreKind;

/// One subscription CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionCliSpec {
    pub cli: SubscriptionCli,
    /// The executable name looked up on PATH.
    pub binary: &'static str,
    /// The product name in user-facing copy (`Codex CLI`).
    pub label: &'static str,
    /// The account the user signs in to (`ChatGPT`).
    pub account_label: &'static str,
    pub status_args: &'static [&'static str],
    pub sign_in_args: &'static [&'static str],
    pub sign_out_args: &'static [&'static str],
    /// Lower-case phrases in the status output that mean "signed out".
    pub signed_out_markers: &'static [&'static str],
    pub winget_package: &'static str,
    /// The vendor's PowerShell bootstrap, run only after its own confirmation.
    pub vendor_script: &'static str,
    /// Where the CLI keeps its login cache.
    pub credential_store: CredentialStoreKind,
}

pub const CODEX_CLI_SPEC: SubscriptionCliSpec = SubscriptionCliSpec {
    cli: SubscriptionCli::Codex,
    binary: "codex",
    label: "Codex CLI",
    account_label: "ChatGPT",
    status_args: &["login", "status"],
    sign_in_args: &["login"],
    sign_out_args: &["logout"],
    signed_out_markers: &["not logged in"],
    winget_package: "OpenAI.Codex",
    vendor_script: "$env:CODEX_NON_INTERACTIVE = '1'; irm https://chatgpt.com/codex/install.ps1 | iex",
    credential_store: CredentialStoreKind::CodexAuthJson,
};

pub const CLAUDE_CODE_SPEC: SubscriptionCliSpec = SubscriptionCliSpec {
    cli: SubscriptionCli::ClaudeCode,
    binary: "claude",
    label: "Claude Code",
    account_label: "Claude",
    status_args: &["auth", "status"],
    sign_in_args: &["auth", "login"],
    sign_out_args: &["auth", "logout"],
    signed_out_markers: &["not logged in", "please run /login"],
    winget_package: "Anthropic.ClaudeCode",
    vendor_script: "irm https://claude.ai/install.ps1 | iex",
    credential_store: CredentialStoreKind::ClaudeCredentialsJson,
};

/// The spec for a CLI.
#[must_use]
pub const fn subscription_cli_spec(cli: SubscriptionCli) -> &'static SubscriptionCliSpec {
    match cli {
        SubscriptionCli::Codex => &CODEX_CLI_SPEC,
        SubscriptionCli::ClaudeCode => &CLAUDE_CODE_SPEC,
    }
}

/// What the vendor's status command said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusVerdict {
    SignedIn,
    SignedOut,
    /// The command failed without a signed-out phrase: never reported as
    /// signed out.
    Unclear,
}

/// Read the status command's outcome. A signed-out marker anywhere wins;
/// otherwise a clean exit means signed in and a failure means unclear.
#[must_use]
pub fn parse_status_output(
    spec: &SubscriptionCliSpec,
    exit_ok: bool,
    stdout: &str,
    stderr: &str,
) -> StatusVerdict {
    let text = format!("{stdout}\n{stderr}").to_lowercase();
    if spec
        .signed_out_markers
        .iter()
        .any(|marker| text.contains(marker))
    {
        StatusVerdict::SignedOut
    } else if exit_ok {
        StatusVerdict::SignedIn
    } else {
        StatusVerdict::Unclear
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_spec_per_cli_pins_the_vendor_commands_and_packages() {
        let codex = subscription_cli_spec(SubscriptionCli::Codex);
        assert_eq!(codex.binary, "codex");
        assert_eq!(codex.status_args, ["login", "status"]);
        assert_eq!(codex.sign_in_args, ["login"]);
        assert_eq!(codex.sign_out_args, ["logout"]);
        assert_eq!(codex.winget_package, "OpenAI.Codex");
        assert_eq!(codex.credential_store, CredentialStoreKind::CodexAuthJson);
        let claude = subscription_cli_spec(SubscriptionCli::ClaudeCode);
        assert_eq!(claude.binary, "claude");
        assert_eq!(claude.status_args, ["auth", "status"]);
        assert_eq!(claude.sign_in_args, ["auth", "login"]);
        assert_eq!(claude.sign_out_args, ["auth", "logout"]);
        assert_eq!(claude.winget_package, "Anthropic.ClaudeCode");
        assert_eq!(
            claude.credential_store,
            CredentialStoreKind::ClaudeCredentialsJson
        );
        for spec in [codex, claude] {
            assert!(spec.vendor_script.starts_with("irm ") || spec.vendor_script.contains("irm "));
            assert!(!spec.signed_out_markers.is_empty());
            assert!(
                spec.signed_out_markers
                    .iter()
                    .all(|m| *m == m.to_lowercase())
            );
        }
    }

    #[test]
    fn status_parser_is_conservative_and_case_insensitive() {
        let claude = subscription_cli_spec(SubscriptionCli::ClaudeCode);
        assert_eq!(
            parse_status_output(claude, true, "Logged in as: mike@example.com", ""),
            StatusVerdict::SignedIn
        );
        assert_eq!(
            parse_status_output(claude, true, "NOT LOGGED IN · Please run /login", ""),
            StatusVerdict::SignedOut
        );
        assert_eq!(
            parse_status_output(claude, false, "", "not logged in"),
            StatusVerdict::SignedOut,
            "a marker on stderr counts"
        );
        let codex = subscription_cli_spec(SubscriptionCli::Codex);
        assert_eq!(
            parse_status_output(codex, false, "Logged in using ChatGPT", ""),
            StatusVerdict::Unclear,
            "a failing command is never promoted to signed in"
        );
    }
}
