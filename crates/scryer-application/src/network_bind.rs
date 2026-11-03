//! The HTTP listening address.
//!
//! Resolution is `SCRYER_BIND`, then the saved host, then loopback. The saved
//! setting holds a host only; its port is `SCRYER_BIND`'s, the `--port`
//! argument, or the default. The address is read once before the listener
//! binds, so a saved change applies on the next start.
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::error_reasons::network as reasons;
use crate::public_url::ConfigValueSource;
use crate::{AppError, AppResult};

pub const BIND_HOST_KEY: &str = "network.bind_host";
pub const BIND_ENV: &str = "SCRYER_BIND";
pub const DEFAULT_PORT: u16 = 8080;
pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// The host the "other computers" choice saves: every IPv4 interface, which
/// still answers on loopback.
pub const NETWORK_WIDE_HOST: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// Why the listening address cannot be changed from the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindPin {
    /// `SCRYER_BIND` sets it.
    Environment,
    /// `SCRYER_BIND` sets it inside a container, whose published ports decide reach.
    Container,
}

/// Why the process is not listening where its configuration said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindFallback {
    /// A `network` area reason.
    pub code: &'static str,
    /// The address that was configured, when there was one.
    pub attempted: Option<SocketAddr>,
    /// Operating-system or parse detail, for display only.
    pub detail: String,
}

/// The first-run answer to who may reach this instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialNetworkAccess {
    ThisComputer,
    OtherComputers,
}

impl InitialNetworkAccess {
    pub fn host(self) -> IpAddr {
        match self {
            Self::ThisComputer => DEFAULT_HOST,
            Self::OtherComputers => NETWORK_WIDE_HOST,
        }
    }
}

/// What the first-run network choice did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InitialBindOutcome {
    /// The environment pins the address, so the choice was not saved.
    pub ignored_because_pinned: bool,
    /// The saved choice takes effect after a restart.
    pub restart_required: bool,
}

/// The startup resolution, before the listener binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindResolution {
    pub address: SocketAddr,
    pub source: ConfigValueSource,
    /// Port a saved host is served on.
    pub port: u16,
    /// A valid saved host, shown even while the environment pins the address.
    pub saved_host: Option<IpAddr>,
    /// A saved value that could not be used.
    pub fallback: Option<BindFallback>,
}

/// Resolve the listening address from its three sources. An invalid
/// `SCRYER_BIND` is an operator error and stops startup; an invalid saved
/// value never does.
pub fn resolve_bind(
    environment: Option<&str>,
    saved: Result<Option<&str>, &str>,
    port_argument: Option<u16>,
) -> Result<BindResolution, String> {
    let environment = environment.map(str::trim).filter(|value| !value.is_empty());
    let (saved_host, saved_fallback) = match saved {
        Ok(None) => (None, None),
        Ok(Some(raw)) => match parse_bind_host(raw) {
            Ok(host) => (Some(host), None),
            Err(_) => (
                None,
                Some(BindFallback {
                    code: reasons::BIND_STORED_HOST_INVALID,
                    attempted: None,
                    detail: format!("the saved listening address {raw:?} is not an IP address"),
                }),
            ),
        },
        Err(detail) => (
            None,
            Some(BindFallback {
                code: reasons::BIND_STORED_HOST_UNREADABLE,
                attempted: None,
                detail: detail.to_string(),
            }),
        ),
    };
    if let Some(raw) = environment {
        let address: SocketAddr = raw
            .parse()
            .map_err(|error| format!("invalid {BIND_ENV} value {raw:?}: {error}"))?;
        return Ok(BindResolution {
            address,
            source: ConfigValueSource::Environment,
            port: address.port(),
            saved_host,
            // The saved value is not in play while the environment pins the address.
            fallback: None,
        });
    }
    let port = port_argument.unwrap_or(DEFAULT_PORT);
    let (host, source) = match saved_host {
        Some(host) => (host, ConfigValueSource::Settings),
        None => (DEFAULT_HOST, ConfigValueSource::Default),
    };
    Ok(BindResolution {
        address: SocketAddr::new(host, port),
        source,
        port,
        saved_host,
        fallback: saved_fallback,
    })
}

/// Parse an operator-entered host: a bare IPv4 or IPv6 address, brackets allowed.
pub fn parse_bind_host(raw: &str) -> AppResult<IpAddr> {
    let trimmed = raw.trim();
    let unbracketed = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    unbracketed.parse::<IpAddr>().map_err(|_| {
        AppError::validation_refused(
            reasons::BIND_HOST_INVALID,
            "enter an IPv4 or IPv6 address without a port, such as 127.0.0.1 or 0.0.0.0",
        )
    })
}

/// The address a failed non-loopback bind retries on. A loopback address has
/// nothing safer to fall back to.
pub fn fallback_address(desired: SocketAddr) -> Option<SocketAddr> {
    (!desired.ip().is_loopback()).then(|| SocketAddr::new(DEFAULT_HOST, desired.port()))
}

/// Whether a host is reachable only from this computer.
pub fn is_loopback_host(host: IpAddr) -> bool {
    host.to_canonical().is_loopback()
}

/// The listening address facts shown to administrators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindState {
    /// Where the listener actually listens.
    pub listening: SocketAddr,
    /// What startup resolved and tried first.
    pub desired: SocketAddr,
    pub source: ConfigValueSource,
    pub pinned_by: Option<BindPin>,
    /// Port a saved host is served on.
    pub port: u16,
    /// The saved host now; updated when an administrator saves.
    pub saved_host: Option<IpAddr>,
    pub fallback: Option<BindFallback>,
}

impl Default for BindState {
    fn default() -> Self {
        let address = SocketAddr::new(DEFAULT_HOST, DEFAULT_PORT);
        Self {
            listening: address,
            desired: address,
            source: ConfigValueSource::Default,
            pinned_by: None,
            port: DEFAULT_PORT,
            saved_host: None,
            fallback: None,
        }
    }
}

impl BindState {
    pub fn from_resolution(
        resolution: &BindResolution,
        listening: SocketAddr,
        pinned_by: Option<BindPin>,
        bind_fallback: Option<BindFallback>,
    ) -> Self {
        Self {
            listening,
            desired: resolution.address,
            source: resolution.source,
            pinned_by: if resolution.source == ConfigValueSource::Environment {
                Some(pinned_by.unwrap_or(BindPin::Environment))
            } else {
                None
            },
            port: resolution.port,
            saved_host: resolution.saved_host,
            fallback: bind_fallback.or_else(|| resolution.fallback.clone()),
        }
    }

    pub fn editable(&self) -> bool {
        self.pinned_by.is_none()
    }

    /// The address the next start resolves.
    pub fn pending(&self) -> SocketAddr {
        if self.pinned_by.is_some() {
            return self.desired;
        }
        SocketAddr::new(self.saved_host.unwrap_or(DEFAULT_HOST), self.port)
    }

    /// Whether the next start resolves a different address than this one did.
    /// Parsed addresses compare, so `::1` and `0:0:0:0:0:0:0:1` are the same.
    /// A fallback does not count: restarting would only fall back again.
    pub fn restart_required(&self) -> bool {
        self.pending() != self.desired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(raw: &str) -> SocketAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn bind_environment_wins_the_whole_address_over_setting_and_port() {
        let resolved = resolve_bind(Some(" 0.0.0.0:9000 "), Ok(Some("192.0.2.4")), Some(7000))
            .expect("valid environment");
        assert_eq!(resolved.address, addr("0.0.0.0:9000"));
        assert_eq!(resolved.source, ConfigValueSource::Environment);
        assert_eq!(resolved.saved_host, Some("192.0.2.4".parse().unwrap()));
        assert!(resolved.fallback.is_none());
        assert!(resolve_bind(Some("not-an-address"), Ok(None), None).is_err());
    }

    #[test]
    fn bind_saved_host_uses_the_port_argument_or_default() {
        let resolved = resolve_bind(None, Ok(Some("0.0.0.0")), Some(7000)).unwrap();
        assert_eq!(resolved.address, addr("0.0.0.0:7000"));
        assert_eq!(resolved.source, ConfigValueSource::Settings);
        let resolved = resolve_bind(Some("  "), Ok(Some("[::]")), None).unwrap();
        assert_eq!(resolved.address, addr("[::]:8080"));
        assert_eq!(resolved.source, ConfigValueSource::Settings);
    }

    #[test]
    fn bind_default_is_loopback_and_bad_saved_values_fall_back_with_a_code() {
        let resolved = resolve_bind(None, Ok(None), None).unwrap();
        assert_eq!(resolved.address, addr("127.0.0.1:8080"));
        assert_eq!(resolved.source, ConfigValueSource::Default);
        assert!(resolved.fallback.is_none());

        let invalid = resolve_bind(None, Ok(Some("example.test")), Some(7000)).unwrap();
        assert_eq!(invalid.address, addr("127.0.0.1:7000"));
        assert_eq!(invalid.source, ConfigValueSource::Default);
        assert_eq!(
            invalid.fallback.unwrap().code,
            reasons::BIND_STORED_HOST_INVALID
        );

        let unreadable = resolve_bind(None, Err("database offline"), None).unwrap();
        assert_eq!(unreadable.address, addr("127.0.0.1:8080"));
        assert_eq!(
            unreadable.fallback.unwrap().code,
            reasons::BIND_STORED_HOST_UNREADABLE
        );
    }

    #[test]
    fn bind_host_validation_accepts_addresses_and_refuses_everything_else() {
        for (raw, expected) in [
            ("127.0.0.1", "127.0.0.1"),
            (" 0.0.0.0 ", "0.0.0.0"),
            ("::", "::"),
            ("[::1]", "::1"),
            ("192.0.2.10", "192.0.2.10"),
        ] {
            assert_eq!(
                parse_bind_host(raw).unwrap(),
                expected.parse::<IpAddr>().unwrap()
            );
        }
        for raw in ["", "localhost", "0.0.0.0:8080", "[::]:8080", "300.0.0.1"] {
            assert_eq!(
                parse_bind_host(raw).unwrap_err().validation_reason(),
                Some(reasons::BIND_HOST_INVALID),
                "{raw}"
            );
        }
    }

    #[test]
    fn bind_fallback_retries_loopback_only_for_wider_addresses() {
        assert_eq!(
            fallback_address(addr("192.0.2.10:7000")),
            Some(addr("127.0.0.1:7000"))
        );
        assert_eq!(
            fallback_address(addr("0.0.0.0:7000")),
            Some(addr("127.0.0.1:7000"))
        );
        assert_eq!(fallback_address(addr("127.0.0.1:7000")), None);
        assert_eq!(fallback_address(addr("[::1]:7000")), None);
    }

    #[test]
    fn bind_restart_required_compares_parsed_addresses() {
        let resolution = resolve_bind(None, Ok(Some("::1")), Some(7000)).unwrap();
        let mut state = BindState::from_resolution(&resolution, resolution.address, None, None);
        assert!(!state.restart_required());
        state.saved_host = Some("0:0:0:0:0:0:0:1".parse().unwrap());
        assert!(!state.restart_required());
        state.saved_host = Some(NETWORK_WIDE_HOST);
        assert_eq!(state.pending(), addr("0.0.0.0:7000"));
        assert!(state.restart_required());
        state.saved_host = None;
        assert_eq!(state.pending(), addr("127.0.0.1:7000"));
        assert!(state.restart_required());
    }

    #[test]
    fn bind_pinned_state_never_waits_for_a_restart() {
        let resolution = resolve_bind(Some("0.0.0.0:8080"), Ok(None), None).unwrap();
        let mut state = BindState::from_resolution(
            &resolution,
            resolution.address,
            Some(BindPin::Container),
            None,
        );
        assert_eq!(state.pinned_by, Some(BindPin::Container));
        assert!(!state.editable());
        state.saved_host = Some(DEFAULT_HOST);
        assert_eq!(state.pending(), addr("0.0.0.0:8080"));
        assert!(!state.restart_required());

        let unpinned = resolve_bind(None, Ok(None), None).unwrap();
        let state =
            BindState::from_resolution(&unpinned, unpinned.address, Some(BindPin::Container), None);
        assert_eq!(state.pinned_by, None);
    }

    #[test]
    fn bind_fallback_after_an_unbindable_address_does_not_ask_for_a_restart() {
        let resolution = resolve_bind(None, Ok(Some("192.0.2.10")), Some(7000)).unwrap();
        let state = BindState::from_resolution(
            &resolution,
            addr("127.0.0.1:7000"),
            None,
            Some(BindFallback {
                code: reasons::BIND_FALLBACK_UNBINDABLE,
                attempted: Some(resolution.address),
                detail: "address not available".into(),
            }),
        );
        assert_eq!(state.listening, addr("127.0.0.1:7000"));
        assert!(!state.restart_required());
        assert_eq!(
            state.fallback.unwrap().code,
            reasons::BIND_FALLBACK_UNBINDABLE
        );
    }

    #[test]
    fn bind_loopback_hosts_include_mapped_loopback() {
        assert!(is_loopback_host("127.0.0.1".parse().unwrap()));
        assert!(is_loopback_host("::1".parse().unwrap()));
        assert!(is_loopback_host("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback_host("0.0.0.0".parse().unwrap()));
        assert!(!is_loopback_host("::".parse().unwrap()));
    }
}
