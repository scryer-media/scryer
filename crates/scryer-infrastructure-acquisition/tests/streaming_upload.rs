use std::sync::{Arc, Mutex};

use scryer_outbound_http::{OutboundHttpClient, RateLimitRegistry, RequestPolicy};
use tokio_util::io::ReaderStream;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn dispatch_observer_allows_a_non_clonable_streaming_upload_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("streamed nzb payload"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let upload = tempfile::NamedTempFile::new().expect("create staged upload");
    std::fs::write(upload.path(), "streamed nzb payload").expect("write staged upload");
    let file = tokio::fs::File::open(upload.path())
        .await
        .expect("open staged upload");
    let file = Arc::new(Mutex::new(Some(file)));

    let client = scryer_outbound_http::generic_reqwest_client();
    let outbound = OutboundHttpClient::new(client.clone(), RateLimitRegistry::isolated());
    let upload_url = server.uri();
    let response = outbound
        .send_with_dispatch_observer(
            RequestPolicy::no_retry("streaming-upload", "staged-nzb-upload"),
            {
                let file = Arc::clone(&file);
                move || {
                    let file = file
                        .lock()
                        .expect("streaming upload file lock")
                        .take()
                        .expect("request builder runs once without retries");
                    client
                        .post(&upload_url)
                        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
                }
            },
            |_| Ok(()),
        )
        .await
        .expect("non-clonable streaming request should be dispatched");

    assert!(response.status().is_success());
}
