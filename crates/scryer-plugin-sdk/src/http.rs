//! The source-level HTTP request shape used by Scryer plugins.
//!
//! Only the builder lives here: this crate carries the request *shape*, not a
//! transport. Guests issue HTTP through `scryer-plugin-pdk`'s `http::request`,
//! which carries this same source shape over the postcard host-call contract
//! (`scryer:host/services@1.0.0`), the only door Scryer's component host
//! serves.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct HttpRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl HttpRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: None,
            headers: BTreeMap::new(),
        }
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::HttpRequest;

    #[test]
    fn http_request_builder_serializes_to_legacy_shape() {
        let request = HttpRequest::new("https://indexer.example/api")
            .with_method("POST")
            .with_header("X-Test", "one");

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["url"], "https://indexer.example/api");
        assert_eq!(json["method"], "POST");
        assert_eq!(json["headers"]["X-Test"], "one");
    }
}
