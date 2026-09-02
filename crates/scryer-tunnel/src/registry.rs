//! One loopback front per proxy configuration, started lazily.
//!
//! ## Why `ensure_*` is synchronous and never blocks
//!
//! The egress seam it serves (`transport_proxy::proxy_egress_url`) is a
//! synchronous function called from three very different places: an async
//! WASI-p2 host, an async download router, and a **blocking** plugin HTTP
//! worker thread that has no tokio runtime at all. Blocking on a future would
//! be wrong in all three (it panics inside a runtime and stalls a worker
//! outside one).
//!
//! So starting a tunnel does no async work: it binds a `std::net::TcpListener`
//! on `127.0.0.1:0` — which is immediate — reads the port off it, and hands the
//! accept loop to a tokio runtime this registry owns. The SSH session is
//! established later, by the first connection the front accepts, inside that
//! runtime. Callers get a port back in microseconds and never touch a future.
//!
//! ## Lifecycle
//!
//! Keyed by proxy config id; the entry carries the configuration revision
//! (`id@updated_at`). Same revision reuses the front, a changed revision stops
//! the old one and starts a new one, so an operator edit takes effect on the
//! next request and a health flap does not churn sessions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::error::TunnelError;
use crate::provider::{TunnelObserver, TunnelProvider, TunnelSpec};
use crate::socks5::Socks5Front;
use crate::ssh::SshTunnelProvider;
use crate::wireguard::{WireGuardSpec, WireGuardTunnelProvider};

struct TunnelEntry {
    revision: String,
    front_addr: SocketAddr,
    #[allow(dead_code)]
    provider: Arc<dyn TunnelProvider>,
    shutdown: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl TunnelEntry {
    fn stop(self) {
        self.shutdown.notify_waiters();
        // The accept loop wakes on the notification; aborting as well makes the
        // stop synchronous from the caller's point of view even when the loop
        // is parked in `accept`.
        self.task.abort();
    }
}

enum RuntimeHome {
    Owned(tokio::runtime::Runtime),
    Borrowed(tokio::runtime::Handle),
}

impl RuntimeHome {
    fn handle(&self) -> tokio::runtime::Handle {
        match self {
            RuntimeHome::Owned(runtime) => runtime.handle().clone(),
            RuntimeHome::Borrowed(handle) => handle.clone(),
        }
    }
}

/// The process-wide set of running tunnels.
pub struct TunnelRegistry {
    runtime: RuntimeHome,
    entries: Mutex<HashMap<String, TunnelEntry>>,
}

static SHARED: LazyLock<std::io::Result<TunnelRegistry>> = LazyLock::new(|| {
    // A small dedicated runtime rather than the ambient one: the blocking
    // plugin host has no ambient runtime, and a tunnel must outlive any single
    // request's runtime context.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("scryer-tunnel")
        .enable_all()
        .build()
        .map(|runtime| TunnelRegistry {
            runtime: RuntimeHome::Owned(runtime),
            entries: Mutex::new(HashMap::new()),
        })
});

impl TunnelRegistry {
    /// The process-wide registry, creating its runtime on first use.
    pub fn shared() -> Result<&'static TunnelRegistry, TunnelError> {
        SHARED.as_ref().map_err(|error| {
            TunnelError::Engine(format!("could not start the tunnel runtime: {error}"))
        })
    }

    /// A registry that runs its tunnels on an existing runtime. For tests, and
    /// for any embedder that would rather own the threads.
    pub fn with_handle(handle: tokio::runtime::Handle) -> Self {
        Self {
            runtime: RuntimeHome::Borrowed(handle),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Ensure an SSH tunnel for `spec` is running and return the loopback
    /// address of its SOCKS5 front.
    pub fn ensure_ssh_tunnel(
        &self,
        spec: TunnelSpec,
        observer: Arc<dyn TunnelObserver>,
    ) -> Result<SocketAddr, TunnelError> {
        let id = spec.proxy_config_id.clone();
        let revision = spec.revision.clone();
        let timeout = spec.request_timeout;
        let for_observer = Arc::clone(&observer);
        self.ensure_tunnel_with(&id, &revision, timeout, observer, move || {
            Ok(Arc::new(SshTunnelProvider::new(spec, for_observer)) as Arc<dyn TunnelProvider>)
        })
    }

    /// Ensure a WireGuard tunnel for `spec` is running and return the loopback
    /// address of its SOCKS5 front.
    ///
    /// The factory below is synchronous, as it must be, and building a
    /// [`WireGuardTunnelProvider`] does no I/O: the device, its handshake and
    /// the smoltcp stack all come up inside the first dial. That is the same
    /// choice the SSH family makes and for the same reason — this call is
    /// reached from a blocking plugin worker with no runtime — and it has the
    /// property that matters here: a failed bring-up leaves the provider
    /// empty, so it is reported to the observer by the dial that hit it and
    /// retried by the next one, rather than poisoning the entry until a
    /// restart.
    pub fn ensure_wireguard_tunnel(
        &self,
        spec: WireGuardSpec,
        observer: Arc<dyn TunnelObserver>,
    ) -> Result<SocketAddr, TunnelError> {
        let id = spec.proxy_config_id.clone();
        let revision = spec.revision.clone();
        let timeout = spec.request_timeout;
        let for_observer = Arc::clone(&observer);
        self.ensure_tunnel_with(&id, &revision, timeout, observer, move || {
            Ok(Arc::new(WireGuardTunnelProvider::new(spec, for_observer))
                as Arc<dyn TunnelProvider>)
        })
    }

    /// The technology-independent half: everything except which provider gets
    /// built. Both `ensure_*` entry points call this with a different factory
    /// and inherit the front, the keying, the lifecycle and every consumer
    /// unchanged.
    pub fn ensure_tunnel_with<F>(
        &self,
        proxy_config_id: &str,
        revision: &str,
        timeout: Duration,
        observer: Arc<dyn TunnelObserver>,
        make_provider: F,
    ) -> Result<SocketAddr, TunnelError>
    where
        F: FnOnce() -> Result<Arc<dyn TunnelProvider>, TunnelError>,
    {
        let mut entries = self.entries.lock().expect("tunnel registry lock poisoned");
        if let Some(existing) = entries.get(proxy_config_id) {
            if existing.revision == revision && !existing.task.is_finished() {
                return Ok(existing.front_addr);
            }
            if let Some(stale) = entries.remove(proxy_config_id) {
                stale.stop();
            }
        }

        let provider = make_provider()?;
        // Bound and read synchronously; only the accept loop needs a runtime.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
            TunnelError::Engine(format!("could not bind a loopback tunnel front: {error}"))
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            TunnelError::Engine(format!("could not prepare the tunnel front: {error}"))
        })?;
        let front_addr = listener.local_addr().map_err(|error| {
            TunnelError::Engine(format!("could not read the tunnel front address: {error}"))
        })?;

        let shutdown = Arc::new(tokio::sync::Notify::new());
        let front = Arc::new(Socks5Front::new(
            Arc::clone(&provider),
            observer,
            proxy_config_id.to_string(),
            timeout,
        ));
        let shutdown_for_task = Arc::clone(&shutdown);
        let task = self.runtime.handle().spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::warn!(error = %error, "could not adopt the tunnel front listener");
                    return;
                }
            };
            front.serve(listener, shutdown_for_task).await;
        });

        tracing::info!(
            proxy_config_id,
            revision,
            front = %front_addr,
            tunnel = provider.describe().as_str(),
            "started a tunnel front"
        );
        entries.insert(
            proxy_config_id.to_string(),
            TunnelEntry {
                revision: revision.to_string(),
                front_addr,
                provider,
                shutdown,
                task,
            },
        );
        Ok(front_addr)
    }

    /// Stop one tunnel, closing its front and dropping its session.
    pub fn stop(&self, proxy_config_id: &str) {
        let entry = self
            .entries
            .lock()
            .expect("tunnel registry lock poisoned")
            .remove(proxy_config_id);
        if let Some(entry) = entry {
            entry.stop();
        }
    }

    /// Stop every tunnel. Called on process shutdown.
    pub fn stop_all(&self) {
        let entries: Vec<TunnelEntry> = self
            .entries
            .lock()
            .expect("tunnel registry lock poisoned")
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        for entry in entries {
            entry.stop();
        }
    }

    /// The front address of a running tunnel, without starting one. Tests only.
    pub fn front_addr(&self, proxy_config_id: &str) -> Option<SocketAddr> {
        self.entries
            .lock()
            .expect("tunnel registry lock poisoned")
            .get(proxy_config_id)
            .map(|entry| entry.front_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TunnelError;
    use crate::provider::{NoopTunnelObserver, TunnelStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct CountingProvider;

    #[async_trait::async_trait]
    impl TunnelProvider for CountingProvider {
        async fn dial(&self, host: &str, port: u16) -> Result<Box<dyn TunnelStream>, TunnelError> {
            Err(TunnelError::Dial {
                host: host.to_string(),
                port,
                detail: "counting provider never dials".to_string(),
            })
        }
        fn describe(&self) -> String {
            "counting provider".to_string()
        }
    }

    fn registry() -> TunnelRegistry {
        TunnelRegistry::with_handle(tokio::runtime::Handle::current())
    }

    fn factory(
        built: Arc<AtomicUsize>,
    ) -> impl FnOnce() -> Result<Arc<dyn TunnelProvider>, TunnelError> {
        move || {
            built.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(CountingProvider) as Arc<dyn TunnelProvider>)
        }
    }

    #[tokio::test]
    async fn the_same_revision_reuses_one_front_and_a_new_revision_restarts_it() {
        let registry = registry();
        let built = Arc::new(AtomicUsize::new(0));

        let first = registry
            .ensure_tunnel_with(
                "proxy-1",
                "proxy-1@v1",
                Duration::from_secs(5),
                Arc::new(NoopTunnelObserver),
                factory(Arc::clone(&built)),
            )
            .expect("first start");
        let again = registry
            .ensure_tunnel_with(
                "proxy-1",
                "proxy-1@v1",
                Duration::from_secs(5),
                Arc::new(NoopTunnelObserver),
                factory(Arc::clone(&built)),
            )
            .expect("reuse");
        assert_eq!(first, again, "the same revision must reuse the front");
        assert_eq!(built.load(Ordering::SeqCst), 1, "no second provider built");

        let restarted = registry
            .ensure_tunnel_with(
                "proxy-1",
                "proxy-1@v2",
                Duration::from_secs(5),
                Arc::new(NoopTunnelObserver),
                factory(Arc::clone(&built)),
            )
            .expect("restart");
        assert_ne!(first, restarted, "an edited proxy gets a new front");
        assert_eq!(built.load(Ordering::SeqCst), 2);

        // The old front is gone.
        tokio::task::yield_now().await;
        assert!(
            connection_is_dead(first).await,
            "the superseded front must stop serving"
        );
    }

    /// A stopped listener either refuses the connection outright or accepts a
    /// pending one that immediately reads EOF; both prove it is not serving.
    async fn connection_is_dead(addr: SocketAddr) -> bool {
        match tokio::net::TcpStream::connect(addr).await {
            Err(_) => true,
            Ok(mut stream) => {
                if stream.write_all(&[0x05, 1, 0x00]).await.is_err() {
                    return true;
                }
                let mut answer = [0u8; 2];
                tokio::time::timeout(Duration::from_millis(250), stream.read_exact(&mut answer))
                    .await
                    .map(|result| result.is_err())
                    .unwrap_or(true)
            }
        }
    }

    #[tokio::test]
    async fn a_front_binds_loopback_on_an_ephemeral_port() {
        let registry = registry();
        let addr = registry
            .ensure_tunnel_with(
                "proxy-loopback",
                "proxy-loopback@v1",
                Duration::from_secs(5),
                Arc::new(NoopTunnelObserver),
                factory(Arc::new(AtomicUsize::new(0))),
            )
            .expect("start");
        assert!(addr.ip().is_loopback(), "{addr}");
        assert_ne!(addr.port(), 0);
        assert_eq!(registry.front_addr("proxy-loopback"), Some(addr));
    }

    #[tokio::test]
    async fn stopping_a_tunnel_closes_its_front() {
        let registry = registry();
        let addr = registry
            .ensure_tunnel_with(
                "proxy-stop",
                "proxy-stop@v1",
                Duration::from_secs(5),
                Arc::new(NoopTunnelObserver),
                factory(Arc::new(AtomicUsize::new(0))),
            )
            .expect("start");
        registry.stop("proxy-stop");
        assert_eq!(registry.front_addr("proxy-stop"), None);
        tokio::task::yield_now().await;
        assert!(connection_is_dead(addr).await, "front still serving {addr}");
    }

    #[tokio::test]
    async fn stop_all_clears_every_tunnel() {
        let registry = registry();
        for id in ["proxy-a", "proxy-b"] {
            registry
                .ensure_tunnel_with(
                    id,
                    &format!("{id}@v1"),
                    Duration::from_secs(5),
                    Arc::new(NoopTunnelObserver),
                    factory(Arc::new(AtomicUsize::new(0))),
                )
                .expect("start");
        }
        registry.stop_all();
        assert_eq!(registry.front_addr("proxy-a"), None);
        assert_eq!(registry.front_addr("proxy-b"), None);
    }
}
