//! Local CONNECT peer using fixture-only certificates from proxy-tunnels v0.1.0.
use super::*;
use bytes::{Buf, Bytes};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    endpoint: quinn::Endpoint,
    task: tokio::task::JoinHandle<()>,
    connections: Arc<AtomicUsize>,
    targets: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    async fn start() -> Self {
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![
                include_bytes!("../tests/fixtures/http3/server.der")
                    .to_vec()
                    .into(),
            ],
            rustls::pki_types::PrivateKeyDer::Pkcs8(
                include_bytes!("../tests/fixtures/http3/server-key.der")
                    .to_vec()
                    .into(),
            ),
        )
        .unwrap();
        tls.alpn_protocols = vec![b"h3".to_vec()];
        let config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap(),
        ));
        let endpoint = quinn::Endpoint::server(config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let server = endpoint.clone();
        let connections = Arc::new(AtomicUsize::new(0));
        let targets = Arc::new(Mutex::new(Vec::new()));
        let count = connections.clone();
        let recorded = targets.clone();
        let task = tokio::spawn(async move {
            let mut sessions = tokio::task::JoinSet::new();
            while let Some(incoming) = server.accept().await {
                let count = count.clone();
                let recorded = recorded.clone();
                sessions.spawn(async move {
                    let Ok(connection) = incoming.await else { return };
                    count.fetch_add(1, Ordering::SeqCst);
                    let mut connection = h3::server::builder()
                        .build(h3_quinn::Connection::new(connection)).await.unwrap();
                    let mut streams = tokio::task::JoinSet::new();
                    while let Ok(Some(resolver)) = connection.accept().await {
                        let recorded = recorded.clone();
                        streams.spawn(async move {
                            let (request, mut stream) = resolver.resolve_request().await.unwrap();
                            assert_eq!(request.method(), http::Method::CONNECT);
                            assert!(request.uri().scheme().is_none());
                            assert!(request.uri().path_and_query().is_none());
                            recorded.lock().unwrap().push(request.uri().authority().unwrap().to_string());
                            let authenticated = request.headers().get("proxy-authorization")
                                .is_some_and(|value| value == "Basic Zml4dHVyZTpzZWNyZXQ=");
                            stream.send_response(http::Response::builder()
                                .status(if authenticated { 200 } else { 407 }).body(()).unwrap()).await.unwrap();
                            if authenticated {
                                let mut request_bytes = Vec::new();
                                while let Some(mut data) = stream.recv_data().await.unwrap() {
                                    let remaining = data.remaining();
                                    request_bytes.extend_from_slice(&data.copy_to_bytes(remaining));
                                    assert!(request_bytes.len() <= 16384);
                                    if request_bytes.windows(4).any(|window| window == b"\r\n\r\n") { break; }
                                }
                                assert!(request_bytes.starts_with(b"GET /"));
                                assert!(!String::from_utf8_lossy(&request_bytes).to_lowercase().contains("proxy-authorization"));
                                stream.send_data(Bytes::from_static(
                                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\ntunnel!",
                                )).await.unwrap();
                            }
                            stream.finish().await.unwrap();
                        });
                        while streams.try_join_next().is_some() {}
                    }
                });
                while sessions.try_join_next().is_some() {}
            }
        });
        Self {
            endpoint,
            task,
            connections,
            targets,
        }
    }

    fn config(&self, id: &str) -> ProxyConfig {
        let mut config = tests::tunnel_config();
        config.id = id.into();
        config.provider_type = ProxyProviderType::Http3;
        config.base_url = format!(
            "https://localhost:{}",
            self.endpoint.local_addr().unwrap().port()
        );
        config.username_encrypted = Some("fixture".into());
        config.password_encrypted = Some("secret".into());
        config.private_key_encrypted = None;
        config.private_key_passphrase_encrypted = None;
        config.host_key_fingerprint = None;
        config.request_timeout_seconds = 3;
        config
    }

    fn install(&self, config: &ProxyConfig) {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(
                include_bytes!("../tests/fixtures/http3/ca.der")
                    .to_vec()
                    .into(),
            )
            .unwrap();
        let spec = http3_spec(config).unwrap();
        let provider =
            scryer_tunnel::Http3TunnelProvider::with_root_certificates(spec.clone(), roots)
                .unwrap();
        TunnelRegistry::shared()
            .unwrap()
            .ensure_tunnel_with(
                &config.id,
                &spec.revision,
                spec.request_timeout,
                Arc::new(LedgerObserver::new(config).unwrap()),
                scryer_tunnel::Socks5Credentials::new(
                    "local".into(),
                    "fixture-local-secret".into(),
                )
                .unwrap(),
                || Ok(Arc::new(provider)),
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"fixture finished");
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http3_async_and_blocking_clients_reuse_quic_and_resolve_at_the_proxy() {
    let fixture = Fixture::start().await;
    let mut config = fixture.config("h3-consumer-reuse");
    fixture.install(&config);
    let client = crate::transport_proxy::transport_proxied_reqwest_client(&config, "").unwrap();
    for _ in 0..3 {
        assert_eq!(
            client
                .get("http://origin.invalid:8080/")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "tunnel!"
        );
    }
    let blocking_config = config.clone();
    tokio::task::spawn_blocking(move || {
        let client =
            crate::transport_proxy::blocking_transport_proxied_reqwest_client(&blocking_config, "")
                .unwrap();
        assert_eq!(
            client
                .get("http://origin.invalid:8080/")
                .send()
                .unwrap()
                .text()
                .unwrap(),
            "tunnel!"
        );
    })
    .await
    .unwrap();
    assert_eq!(fixture.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fixture.targets.lock().unwrap(),
        vec!["origin.invalid:8080"; 4]
    );
    assert!(TunnelHostKeyLedger::shared().take(&config.id).is_none());
    config.updated_at += chrono::Duration::seconds(1);
    fixture.install(&config);
    assert!(
        client
            .get("http://origin.invalid:8080/")
            .send()
            .await
            .is_err()
    );
    let replacement =
        crate::transport_proxy::transport_proxied_reqwest_client(&config, "").unwrap();
    assert_eq!(
        replacement
            .get("http://origin.invalid:8080/")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(fixture.connections.load(Ordering::SeqCst), 2);
    stop_tunnel(&config.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn http3_rejects_untrusted_proxy_certificates_and_bad_credentials() {
    let fixture = Fixture::start().await;
    let config = fixture.config("h3-untrusted-root");
    let client = crate::transport_proxy::transport_proxied_reqwest_client(&config, "").unwrap();
    assert!(
        client
            .get("http://origin.invalid:8080/")
            .send()
            .await
            .is_err()
    );
    assert!(fixture.targets.lock().unwrap().is_empty());
    stop_tunnel(&config.id);
    let mut config = fixture.config("h3-rejected-auth");
    config.password_encrypted = Some("incorrect".into());
    fixture.install(&config);
    let client = crate::transport_proxy::transport_proxied_reqwest_client(&config, "").unwrap();
    assert!(
        client
            .get("http://origin.invalid:8080/")
            .send()
            .await
            .is_err()
    );
    assert_eq!(fixture.targets.lock().unwrap().len(), 1);
    stop_tunnel(&config.id);
}
