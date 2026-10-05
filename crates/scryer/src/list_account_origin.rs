use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scryer_application::public_url::{PublicUrlPolicy, PublicUrlRuntime};
use scryer_interface::context::{ListAccountLinkOriginSource, RequestListAccountLinkOrigins};
use url::Url;

/// Callback destinations approved by server configuration: the public URL's
/// origin, loopback origins, and this machine's own interface addresses on the
/// port Scryer serves. Request headers cannot enlarge this set, including when
/// the request arrives through a proxy.
#[derive(Clone, Default)]
pub(crate) struct ListAccountOriginPolicy(Arc<PolicyInner>);

#[derive(Default)]
struct PolicyInner {
    public_url: PublicUrlRuntime,
    listener: Option<Listener>,
    interfaces: InterfaceAddresses,
}

#[derive(Clone, Copy)]
struct Listener {
    bind: SocketAddr,
    scheme: &'static str,
}

impl ListAccountOriginPolicy {
    pub(crate) fn from_env(bind: &str, public_url: PublicUrlRuntime) -> Self {
        let tls_enabled = ["SCRYER_TLS_CERT", "SCRYER_TLS_KEY"]
            .iter()
            .all(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()));
        let Ok(bind) = bind.parse() else {
            return Self::default();
        };
        Self::new(public_url, bind, tls_enabled, InterfaceAddresses::system())
    }

    pub(crate) fn new(
        public_url: PublicUrlRuntime,
        bind: SocketAddr,
        tls_enabled: bool,
        interfaces: InterfaceAddresses,
    ) -> Self {
        Self(Arc::new(PolicyInner {
            public_url,
            listener: Some(Listener {
                bind,
                scheme: if tls_enabled { "https" } else { "http" },
            }),
            interfaces,
        }))
    }

    /// The per-request handle. It computes nothing until a resolver links an
    /// account, so ordinary GraphQL requests do not read interfaces or build
    /// origin lists.
    pub(crate) fn request_context(&self) -> RequestListAccountLinkOrigins {
        RequestListAccountLinkOrigins::new(self.0.clone())
    }
}

impl PolicyInner {
    /// Origins approved right now, read from the live public URL so a
    /// saved-setting change applies to the next link attempt.
    async fn approved_origins_now(&self) -> Arc<[String]> {
        let Some(listener) = self.listener else {
            return Arc::from([]);
        };
        let interfaces = if listener.bind.ip().is_unspecified() && listener.bind.port() != 0 {
            self.interfaces.addresses().await
        } else {
            Arc::from([])
        };
        approved_origins(&self.public_url.snapshot(), listener, &interfaces).into()
    }
}

impl ListAccountLinkOriginSource for PolicyInner {
    fn approved_origins(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Arc<[String]>> + Send + '_>> {
        Box::pin(self.approved_origins_now())
    }
}

fn approved_origins(
    public_url: &PublicUrlPolicy,
    listener: Listener,
    interfaces: &[IpAddr],
) -> Vec<String> {
    let mut origins = Vec::new();
    // An invalid public URL falls back to the local origins below; the
    // validation error is logged at startup and shown in settings.
    if let Some(origin) = public_url.origin() {
        origins.push(origin);
    }

    // A wildcard listener approves the addresses of this machine's own
    // interfaces, never arbitrary hostnames that resolve to it. DNS names,
    // published container ports and reverse-proxy addresses require the
    // public URL.
    let Listener { bind, scheme } = listener;
    if bind.port() == 0 {
        return origins;
    }
    if !bind.ip().is_unspecified() {
        push_origin(&mut origins, &format!("{scheme}://{bind}"));
    } else {
        for address in interfaces {
            if !address_reachable_through(*address, bind.ip()) {
                continue;
            }
            let address = SocketAddr::new(*address, bind.port());
            push_origin(&mut origins, &format!("{scheme}://{address}"));
        }
    }
    for host in ["localhost", "127.0.0.1", "[::1]"] {
        push_origin(&mut origins, &format!("{scheme}://{host}:{}", bind.port()));
    }
    origins
}

/// Whether a browser can name this interface address in an origin and reach
/// the wildcard listener through it.
fn address_reachable_through(address: IpAddr, bind: IpAddr) -> bool {
    if address.is_unspecified() || address.is_multicast() {
        return false;
    }
    match (address, bind) {
        (IpAddr::V4(_), IpAddr::V4(_)) => true,
        // Whether an IPv6 wildcard also accepts IPv4 depends on the
        // platform's dual-stack default, which this policy cannot observe:
        // the listener is bound separately at startup. Approve IPv4 interface
        // addresses only for an IPv4 wildcard.
        (IpAddr::V4(_), IpAddr::V6(_)) => false,
        // A browser origin cannot carry an IPv6 zone, so link-local addresses
        // are unusable; an IPv4 wildcard does not accept IPv6 connections.
        (IpAddr::V6(address), IpAddr::V6(_)) => !address.is_unicast_link_local(),
        (IpAddr::V6(_), IpAddr::V4(_)) => false,
    }
}

fn configured_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_some_and(|host| host.contains('*'))
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn push_origin(origins: &mut Vec<String>, value: &str) {
    if let Some(origin) = configured_origin(value)
        && !origins.contains(&origin)
    {
        origins.push(origin);
    }
}

/// This machine's interface addresses. The system source is re-read at most
/// once per refresh interval so a DHCP renewal is picked up without a restart.
#[derive(Clone)]
pub(crate) struct InterfaceAddresses(InterfaceSource);

#[derive(Clone)]
enum InterfaceSource {
    System(Arc<Mutex<Option<(Instant, Arc<[IpAddr]>)>>>),
    #[cfg(test)]
    Fixed(Arc<[IpAddr]>),
}

const INTERFACE_REFRESH: Duration = Duration::from_secs(30);

impl Default for InterfaceAddresses {
    fn default() -> Self {
        Self::system()
    }
}

impl InterfaceAddresses {
    pub(crate) fn system() -> Self {
        Self(InterfaceSource::System(Arc::new(Mutex::new(None))))
    }

    #[cfg(test)]
    pub(crate) fn fixed(addresses: Vec<IpAddr>) -> Self {
        Self(InterfaceSource::Fixed(addresses.into()))
    }

    /// The cache lock is held only to read or swap the cached list; the
    /// enumeration itself runs on the blocking pool, off the async workers.
    async fn addresses(&self) -> Arc<[IpAddr]> {
        match &self.0 {
            #[cfg(test)]
            InterfaceSource::Fixed(addresses) => addresses.clone(),
            InterfaceSource::System(cache) => {
                {
                    let cached = cache.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some((read_at, addresses)) = cached.as_ref()
                        && read_at.elapsed() < INTERFACE_REFRESH
                    {
                        return addresses.clone();
                    }
                }
                let Ok(addresses) = tokio::task::spawn_blocking(system_interface_addresses).await
                else {
                    return Arc::from([]);
                };
                let addresses: Arc<[IpAddr]> = addresses.into();
                *cache.lock().unwrap_or_else(|error| error.into_inner()) =
                    Some((Instant::now(), addresses.clone()));
                addresses
            }
        }
    }
}

#[cfg(unix)]
fn system_interface_addresses() -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a linked list that is released
    // below with freeifaddrs; entries are only read while the list is alive.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return addresses;
    }
    let mut cursor = list;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a non-null node of the list returned above.
        let entry = unsafe { &*cursor };
        if !entry.ifa_addr.is_null() {
            // SAFETY: `ifa_addr` is non-null and its family selects the layout.
            let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
            if family == libc::AF_INET {
                // SAFETY: AF_INET addresses are sockaddr_in.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
                addresses.push(IpAddr::from(
                    u32::from_be(address.sin_addr.s_addr).to_be_bytes(),
                ));
            } else if family == libc::AF_INET6 {
                // SAFETY: AF_INET6 addresses are sockaddr_in6.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                addresses.push(IpAddr::from(address.sin6_addr.s6_addr));
            }
        }
        cursor = entry.ifa_next;
    }
    // SAFETY: `list` came from a successful getifaddrs call.
    unsafe { libc::freeifaddrs(list) };
    addresses
}

/// Without an interface enumeration API, ask the routing table which local
/// address reaches the outside. Connecting a UDP socket sends no packets.
#[cfg(not(unix))]
fn system_interface_addresses() -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    for (bind, target) in [("0.0.0.0:0", "192.0.2.1:9"), ("[::]:0", "[2001:db8::1]:9")] {
        if let Ok(socket) = std::net::UdpSocket::bind(bind)
            && socket.connect(target).is_ok()
            && let Ok(local) = socket.local_addr()
        {
            addresses.push(local.ip());
        }
    }
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_application::public_url::InstanceAddressing;

    fn runtime(public_url: Option<&str>) -> PublicUrlRuntime {
        PublicUrlRuntime::new(
            PublicUrlPolicy::new(public_url, None),
            InstanceAddressing::default(),
        )
    }

    fn origins_with(
        public_url: Option<&str>,
        bind: &str,
        tls_enabled: bool,
        interfaces: Vec<IpAddr>,
    ) -> Vec<String> {
        let policy = ListAccountOriginPolicy::new(
            runtime(public_url),
            bind.parse().expect("bind address"),
            tls_enabled,
            InterfaceAddresses::fixed(interfaces),
        );
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(policy.request_context().approved_origins())
            .to_vec()
    }

    fn origins(public_url: Option<&str>, bind: &str, tls_enabled: bool) -> Vec<String> {
        origins_with(public_url, bind, tls_enabled, Vec::new())
    }

    #[test]
    fn account_link_origin_approves_configured_private_and_proxy_urls_with_base_paths() {
        for (url, expected) in [
            ("https://media.home/scryer/", "https://media.home"),
            ("http://192.168.1.20:8080/base", "http://192.168.1.20:8080"),
            ("https://[fd00::20]:8443/scryer", "https://[fd00::20]:8443"),
            ("https://MEDIA.HOME:443/scryer", "https://media.home"),
        ] {
            let approved = origins(Some(url), "127.0.0.1:8080", false);
            assert!(approved.iter().any(|origin| origin == expected));
            assert!(
                !approved
                    .iter()
                    .any(|origin| origin == "https://attacker.invalid")
            );
        }
    }

    #[test]
    fn account_link_origin_approves_only_literal_bind_and_loopback_defaults() {
        let approved = origins(None, "192.168.1.20:8443", true);
        assert!(approved.contains(&"https://192.168.1.20:8443".into()));
        assert!(approved.contains(&"https://localhost:8443".into()));
        assert!(approved.contains(&"https://127.0.0.1:8443".into()));
        assert!(approved.contains(&"https://[::1]:8443".into()));
        assert!(!approved.contains(&"http://192.168.1.20:8443".into()));
        assert!(!approved.contains(&"https://192.168.1.21:8443".into()));
        assert!(!approved.contains(&"https://media.home:8443".into()));
        assert!(origins(None, "127.0.0.1:80", false).contains(&"http://localhost".into()));
    }

    #[test]
    fn account_link_origin_literal_bind_ignores_other_interfaces() {
        let approved = origins_with(
            None,
            "192.168.1.20:8080",
            false,
            vec!["10.0.0.5".parse().unwrap()],
        );
        assert!(approved.contains(&"http://192.168.1.20:8080".into()));
        assert!(!approved.contains(&"http://10.0.0.5:8080".into()));
    }

    #[test]
    fn account_link_origin_wildcard_bind_approves_machine_addresses_but_not_dns_or_foreign_ones() {
        let interfaces = vec!["192.168.1.20".parse().unwrap(), "fd00::20".parse().unwrap()];
        let v4 = origins_with(None, "0.0.0.0:8080", false, interfaces.clone());
        assert!(v4.contains(&"http://192.168.1.20:8080".into()));
        let v6 = origins_with(None, "[::]:8080", false, interfaces);
        assert!(v6.contains(&"http://[fd00::20]:8080".into()));
        for approved in [&v4, &v6] {
            // Names that resolve to this machine and addresses it does not own
            // need the public URL.
            assert!(!approved.contains(&"http://media.home:8080".into()));
            assert!(!approved.contains(&"http://192.168.1.21:8080".into()));
            assert!(!approved.contains(&"http://172.17.0.1:8080".into()));
            assert!(!approved.contains(&"http://[fd00::21]:8080".into()));
            assert!(approved.contains(&"http://localhost:8080".into()));
        }
        assert!(origins(None, "127.0.0.1:0", false).is_empty());
    }

    #[test]
    fn account_link_origin_wildcard_bind_approves_this_machines_interface_addresses() {
        let interfaces = vec![
            "192.168.1.20".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
            "fd00::20".parse().unwrap(),
            "fe80::1".parse().unwrap(),
        ];
        let v4 = origins_with(None, "0.0.0.0:8080", false, interfaces.clone());
        assert!(v4.contains(&"http://192.168.1.20:8080".into()));
        assert!(!v4.contains(&"http://[fd00::20]:8080".into()));
        assert!(!v4.contains(&"http://192.168.1.21:8080".into()));
        assert!(!v4.contains(&"http://192.168.1.20:9090".into()));
        assert!(!v4.contains(&"https://192.168.1.20:8080".into()));
        assert!(!v4.contains(&"http://media.home:8080".into()));
        assert_eq!(
            v4.iter()
                .filter(|origin| *origin == "http://127.0.0.1:8080")
                .count(),
            1
        );

        // An IPv6 wildcard may or may not accept IPv4 on this platform, so
        // IPv4 interface addresses are not approved for it; loopback stays.
        let v6 = origins_with(None, "[::]:8080", false, interfaces);
        assert!(!v6.contains(&"http://192.168.1.20:8080".into()));
        assert!(v6.contains(&"http://127.0.0.1:8080".into()));
        assert!(v6.contains(&"http://[fd00::20]:8080".into()));
        assert!(!v6.iter().any(|origin| origin.contains("fe80")));
    }

    #[test]
    fn account_link_origin_invalid_public_url_falls_back_to_local_origins() {
        for value in [
            "not a URL",
            "ftp://media.home",
            "https://user:secret@media.home",
            "https://media.home?callback=attacker",
            "https://media.home#callback",
            "https://*.home",
            "*",
        ] {
            assert_eq!(
                origins(Some(value), "127.0.0.1:8080", false),
                origins(None, "127.0.0.1:8080", false),
                "{value}"
            );
            assert!(
                !origins(Some(value), "127.0.0.1:8080", false)
                    .iter()
                    .any(|origin| origin.contains("media.home"))
            );
        }
        assert!(
            origins(Some("not a URL"), "127.0.0.1:8080", false)
                .contains(&"http://127.0.0.1:8080".into())
        );
    }
}
