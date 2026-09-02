//! Network path test: is the router reachable, does the internet answer,
//! does DNS resolve? The Windows collector runs three small probes; the
//! verdict is decided here, portably, from their outcomes.
//!
//! Opt-in only (`AppSettings::network_tests_enabled`): this is the one task
//! that sends anything off the machine, and every target is a compile-time
//! constant listed in the Settings toggle's description.

use serde::Serialize;
use std::time::Duration;

/// The name Windows' own connectivity check (NCSI) resolves, so this adds no
/// new third-party endpoint.
pub const DNS_PROBE_HOST: &str = "www.msftconnecttest.com";
/// Public resolvers that answer TCP 443 from anywhere; two, so one being
/// filtered is not read as "no internet".
pub const TCP_PROBE_TARGETS: [(&str, u16); 2] = [("1.1.1.1", 443), ("8.8.8.8", 443)];
/// Two ICMP echoes to the default gateway, 1 s each.
pub const GATEWAY_PING_ATTEMPTS: u32 = 2;
pub const GATEWAY_PING_TIMEOUT: Duration = Duration::from_secs(1);
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DNS_TIMEOUT: Duration = Duration::from_secs(3);
/// The whole task must finish inside this.
pub const TASK_BUDGET: Duration = Duration::from_secs(6);

/// The outcome of one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Ok,
    Failed,
    /// Could not even be attempted (no gateway, no adapter, timeout setup).
    Skipped,
}

impl ProbeOutcome {
    #[must_use]
    pub const fn from_result(ok: bool) -> Self {
        if ok { Self::Ok } else { Self::Failed }
    }
}

/// What the Windows collector observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Probes {
    /// The IPv4 default gateway of an adapter that is up, when one exists.
    pub gateway: Option<String>,
    pub gateway_ping: ProbeOutcome,
    /// TCP 443 to each of `TCP_PROBE_TARGETS`, in order.
    pub tcp: Vec<(String, ProbeOutcome)>,
    /// Resolving `DNS_PROBE_HOST`.
    pub dns: ProbeOutcome,
    /// True when only IPv6 gateways were found; the verdict stays unknown.
    pub ipv6_only: bool,
}

/// The decision, in the words the issue rules use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Everything answered (or only ICMP was dropped, which routers do).
    Clear,
    /// No adapter is up with an IPv4 gateway.
    NoInternet,
    /// The router did not answer and nothing beyond it did either.
    GatewayUnreachable,
    /// The router answers but nothing beyond it does (WAN down).
    WanDown,
    /// The internet answers but names do not resolve.
    DnsResolutionFailing,
    /// IPv6-only or every probe skipped: not enough evidence to decide.
    Unknown,
}

impl Verdict {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "Connected",
            Self::NoInternet => "No network connection",
            Self::GatewayUnreachable => "Router not reachable",
            Self::WanDown => "Router reachable, internet not",
            Self::DnsResolutionFailing => "Internet reachable, DNS failing",
            Self::Unknown => "Could not decide",
        }
    }
}

/// The task's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkPathReport {
    pub verdict: Verdict,
    pub verdict_label: &'static str,
    pub probes: Probes,
    pub dns_probe_host: &'static str,
    pub tcp_probe_targets: Vec<String>,
}

/// Decide from the probe outcomes.
///
/// | gateway | any TCP 443 | DNS | verdict |
/// | --- | --- | --- | --- |
/// | none (IPv4) | — | — | `NoInternet` (or `Unknown` when IPv6-only) |
/// | failed | failed | — | `GatewayUnreachable` |
/// | ok | failed | — | `WanDown` |
/// | any | ok | failed | `DnsResolutionFailing` |
/// | failed | ok | ok | `Clear` (routers that drop ICMP are not a fault) |
#[must_use]
pub fn assess(probes: Probes) -> NetworkPathReport {
    let internet = probes
        .tcp
        .iter()
        .any(|(_, outcome)| *outcome == ProbeOutcome::Ok);
    let tcp_attempted = probes
        .tcp
        .iter()
        .any(|(_, outcome)| *outcome != ProbeOutcome::Skipped);
    let verdict = if probes.gateway.is_none() {
        if probes.ipv6_only || internet {
            Verdict::Unknown
        } else {
            Verdict::NoInternet
        }
    } else if internet {
        match probes.dns {
            ProbeOutcome::Failed => Verdict::DnsResolutionFailing,
            ProbeOutcome::Ok | ProbeOutcome::Skipped => Verdict::Clear,
        }
    } else if !tcp_attempted {
        Verdict::Unknown
    } else if probes.gateway_ping == ProbeOutcome::Ok {
        Verdict::WanDown
    } else {
        Verdict::GatewayUnreachable
    };
    NetworkPathReport {
        verdict,
        verdict_label: verdict.label(),
        probes,
        dns_probe_host: DNS_PROBE_HOST,
        tcp_probe_targets: TCP_PROBE_TARGETS
            .iter()
            .map(|(host, port)| format!("{host}:{port}"))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probes(
        gateway: Option<&str>,
        gateway_ping: ProbeOutcome,
        tcp: [ProbeOutcome; 2],
        dns: ProbeOutcome,
    ) -> Probes {
        Probes {
            gateway: gateway.map(str::to_string),
            gateway_ping,
            tcp: TCP_PROBE_TARGETS
                .iter()
                .zip(tcp)
                .map(|((host, _), outcome)| (host.to_string(), outcome))
                .collect(),
            dns,
            ipv6_only: false,
        }
    }

    use ProbeOutcome::{Failed, Ok, Skipped};

    #[test]
    fn decision_table() {
        let gateway = Some("192.168.1.1");
        assert_eq!(
            assess(probes(None, Skipped, [Skipped, Skipped], Skipped)).verdict,
            Verdict::NoInternet
        );
        assert_eq!(
            assess(probes(gateway, Failed, [Failed, Failed], Failed)).verdict,
            Verdict::GatewayUnreachable
        );
        assert_eq!(
            assess(probes(gateway, Ok, [Failed, Failed], Failed)).verdict,
            Verdict::WanDown
        );
        assert_eq!(
            assess(probes(gateway, Ok, [Ok, Failed], Failed)).verdict,
            Verdict::DnsResolutionFailing
        );
        assert_eq!(
            assess(probes(gateway, Failed, [Failed, Ok], Ok)).verdict,
            Verdict::Clear,
            "an ICMP-dropping router is not a fault"
        );
        assert_eq!(
            assess(probes(gateway, Ok, [Ok, Ok], Ok)).verdict,
            Verdict::Clear
        );
    }

    #[test]
    fn insufficient_evidence_stays_unknown() {
        let mut ipv6 = probes(None, Skipped, [Skipped, Skipped], Skipped);
        ipv6.ipv6_only = true;
        assert_eq!(assess(ipv6).verdict, Verdict::Unknown);
        assert_eq!(
            assess(probes(
                Some("10.0.0.1"),
                Skipped,
                [Skipped, Skipped],
                Skipped
            ))
            .verdict,
            Verdict::Unknown
        );
        // Internet without a detected gateway (VPN adapters): never "no internet".
        assert_eq!(
            assess(probes(None, Skipped, [Ok, Ok], Ok)).verdict,
            Verdict::Unknown
        );
    }

    #[test]
    fn report_names_every_target_it_contacted() {
        let report = assess(probes(Some("192.168.1.1"), Ok, [Ok, Ok], Ok));
        assert_eq!(report.dns_probe_host, "www.msftconnecttest.com");
        assert_eq!(report.tcp_probe_targets, ["1.1.1.1:443", "8.8.8.8:443"]);
        assert_eq!(report.verdict_label, "Connected");
        assert!(TASK_BUDGET >= GATEWAY_PING_TIMEOUT * GATEWAY_PING_ATTEMPTS + TCP_CONNECT_TIMEOUT);
    }
}
