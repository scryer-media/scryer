//! Filename recovery for obfuscated automatic imports.
//!
//! SABnzbd and NZBGet unpack before Scryer sees the download, so a scene
//! release that shipped its video inside an obfuscated RAR set lands on disk
//! with a name that carries no title signal at all. srrdb.com indexes the
//! *extracted* members of scene releases by CRC-32 and byte size, which is
//! exactly what such a file is.
//!
//! Everything here is a parsing input only. The recovered name never reaches
//! the filesystem: the physical path is what every fs, artifact, move and
//! cleanup call keeps using, and a failure of any kind leaves the import on
//! exactly the path it would take with the feature off.

use crate::ports::{SrrdbFilenameLookup, SrrdbOutage};
use async_trait::async_trait;
use scryer_outbound_http::{
    HostRpsProfile, OutboundHttpClient, RateLimitRegistry, RequestPolicy, RetryMode,
    no_redirect_reqwest_client,
};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

/// Production base URL for the srrdb.com v1 API.
pub const SRRDB_API_BASE_URL: &str = "https://api.srrdb.com/v1";

/// Streaming read buffer, same size as the location-verify copy chunk.
const SRRDB_CRC_CHUNK_BYTES: usize = 1024 * 1024;

/// Whole-call ceiling. srrdb is a best-effort side lookup; an import must never
/// wait on it.
const SRRDB_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// Body caps. A search answer is a handful of rows; a details answer for a big
/// pack lists hundreds of members.
const SRRDB_SEARCH_BODY_CAP_BYTES: usize = 64 * 1024;
const SRRDB_DETAILS_BODY_CAP_BYTES: usize = 1024 * 1024;

/// Recovered names are used as parsing input and compared against real file
/// names; anything longer than a POSIX name is not a filename.
const SRRDB_MAX_RECOVERED_NAME_BYTES: usize = 255;

static SRRDB_RATE_LIMITS: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::new);

/// The two download clients that unpack before Scryer sees the files.
///
/// Weaver does its own recovery internally, torrents are named by the tracker,
/// and plugin clients carry their own type strings; none of them are asked.
pub(crate) fn srrdb_lookup_applies(enabled: bool, client_type: &str) -> bool {
    enabled
        && matches!(
            client_type.trim().to_ascii_lowercase().as_str(),
            "sabnzbd" | "nzbget"
        )
}

/// CRC-32/ISO-HDLC of `path`, plus the number of bytes read.
///
/// Synchronous and streaming: callers run it under `tokio::task::spawn_blocking`.
/// The returned CRC is formatted for the API as `format!("{crc:08X}")`.
pub(crate) fn crc32_iso_hdlc_of_file(path: &Path) -> std::io::Result<(u32, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc32IsoHdlc);
    let mut buffer = vec![0u8; SRRDB_CRC_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    Ok((digest.finalize() as u32, total))
}

#[derive(Debug, Deserialize)]
struct SrrdbSearchResponse {
    #[serde(default)]
    results: Vec<SrrdbSearchResult>,
    #[serde(rename = "resultsCount", default)]
    results_count: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SrrdbSearchResult {
    #[serde(default)]
    release: String,
}

#[derive(Debug, Deserialize)]
struct SrrdbDetailsResponse {
    #[serde(default)]
    name: String,
    #[serde(rename = "archived-files", default)]
    archived_files: Vec<SrrdbArchivedFile>,
}

#[derive(Debug, Deserialize)]
struct SrrdbArchivedFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    crc: String,
}

/// `resultsCount` is a JSON number in the live API, but it has been seen as a
/// string in the wild; accept either and reject anything else.
fn results_count_as_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// A recovered name is a bare file name that Scryer would recognise as video.
///
/// Rejects anything that could escape a directory or confuse a path join, even
/// though the name is never used as a path, because it is compared against and
/// parsed alongside real names.
fn recovered_name_is_acceptable(name: &str) -> bool {
    if name.is_empty() || name.len() > SRRDB_MAX_RECOVERED_NAME_BYTES {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    if name.contains("..") || name.starts_with('.') {
        return false;
    }
    if name.trim() != name || name.trim().is_empty() {
        return false;
    }
    scryer_domain::is_video_file(Path::new(name))
}

/// srrdb.com adapter for [`SrrdbFilenameLookup`].
///
/// Transport mirrors the plugin HTTP runtime: no-redirect client, its own rate
/// limit registry, `NoRetry`, one shared 1 rps lane for the host, and a hard
/// whole-call timeout.
pub struct SrrdbHttpFilenameLookup {
    base_url: reqwest::Url,
    client: OutboundHttpClient,
}

impl SrrdbHttpFilenameLookup {
    pub fn new(base_url: reqwest::Url) -> Self {
        Self {
            base_url,
            client: OutboundHttpClient::new(
                no_redirect_reqwest_client(),
                SRRDB_RATE_LIMITS.clone(),
            ),
        }
    }

    fn policy(label: &'static str) -> RequestPolicy {
        RequestPolicy::new("srrdb", label, RetryMode::NoRetry)
            .without_redirects()
            .with_host_rps_limit("srrdb", HostRpsProfile::limited(1.0, 1))
    }

    /// `<base>/search/archive-crc:<CRC>` with the CRC as a literal path
    /// segment, and `<base>/details/<release>` with the release percent-encoded.
    fn endpoint(&self, prefix: &str, segment: &str) -> Option<reqwest::Url> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url.path_segments_mut().ok()?;
            segments.pop_if_empty();
            segments.push(prefix);
            segments.push(segment);
        }
        Some(url)
    }

    /// Fetch and JSON-decode with a hard body cap.
    ///
    /// `Ok(None)` for every miss-class outcome (redirect, non-2xx that is not
    /// outage-class, oversized body, malformed JSON). `Err(SrrdbOutage)` only
    /// for 429, 5xx, timeout, and transport failures.
    async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: reqwest::Url,
        label: &'static str,
        body_cap_bytes: usize,
    ) -> Result<Option<T>, SrrdbOutage> {
        let request_url = url.clone();
        let send = self.client.send(Self::policy(label), || {
            self.client.client().get(request_url.clone())
        });
        let response = match tokio::time::timeout(SRRDB_REQUEST_TIMEOUT, send).await {
            Err(_) => {
                tracing::debug!(%url, label, "srrdb lookup timed out");
                return Err(SrrdbOutage);
            }
            Ok(Err(error)) => {
                tracing::debug!(%url, label, %error, "srrdb lookup transport failure");
                return Err(SrrdbOutage);
            }
            Ok(Ok(response)) => response,
        };

        let status = response.status();
        if status.is_redirection() {
            tracing::debug!(%url, label, %status, "srrdb lookup redirected; treating as a miss");
            return Ok(None);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            tracing::debug!(%url, label, %status, "srrdb lookup outage status");
            return Err(SrrdbOutage);
        }
        if !status.is_success() {
            tracing::debug!(%url, label, %status, "srrdb lookup non-success status");
            return Ok(None);
        }

        let mut body: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        loop {
            let chunk = match tokio::time::timeout(SRRDB_REQUEST_TIMEOUT, {
                use futures_util::StreamExt;
                stream.next()
            })
            .await
            {
                Err(_) => {
                    tracing::debug!(%url, label, "srrdb body read timed out");
                    return Err(SrrdbOutage);
                }
                Ok(None) => break,
                Ok(Some(Err(error))) => {
                    tracing::debug!(%url, label, %error, "srrdb body read failed");
                    return Err(SrrdbOutage);
                }
                Ok(Some(Ok(chunk))) => chunk,
            };
            if body.len().saturating_add(chunk.len()) > body_cap_bytes {
                tracing::debug!(
                    %url,
                    label,
                    body_cap_bytes,
                    "srrdb response exceeded the body cap; treating as a miss"
                );
                return Ok(None);
            }
            body.extend_from_slice(&chunk);
        }

        match serde_json::from_slice::<T>(&body) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(error) => {
                tracing::debug!(%url, label, %error, "srrdb response was not the expected JSON");
                Ok(None)
            }
        }
    }
}

#[async_trait]
impl SrrdbFilenameLookup for SrrdbHttpFilenameLookup {
    async fn recover_filename(
        &self,
        crc32_hex: &str,
        size_bytes: u64,
    ) -> Result<Option<String>, SrrdbOutage> {
        let Some(search_url) = self.endpoint("search", &format!("archive-crc:{crc32_hex}")) else {
            tracing::debug!(crc32_hex, "srrdb base URL cannot carry path segments");
            return Ok(None);
        };
        let Some(search) = self
            .fetch_json::<SrrdbSearchResponse>(
                search_url,
                "srrdb-search",
                SRRDB_SEARCH_BODY_CAP_BYTES,
            )
            .await?
        else {
            return Ok(None);
        };

        if results_count_as_u64(&search.results_count) != Some(1) || search.results.len() != 1 {
            tracing::debug!(
                crc32_hex,
                results = search.results.len(),
                "srrdb search was not a single unambiguous release"
            );
            return Ok(None);
        }
        let release = search.results[0].release.trim().to_string();
        if release.is_empty() {
            tracing::debug!(crc32_hex, "srrdb search returned an empty release name");
            return Ok(None);
        }

        let Some(details_url) = self.endpoint("details", &release) else {
            tracing::debug!(crc32_hex, "srrdb base URL cannot carry path segments");
            return Ok(None);
        };
        let Some(details) = self
            .fetch_json::<SrrdbDetailsResponse>(
                details_url,
                "srrdb-details",
                SRRDB_DETAILS_BODY_CAP_BYTES,
            )
            .await?
        else {
            return Ok(None);
        };

        if details.name.trim() != release {
            tracing::debug!(
                crc32_hex,
                search_release = %release,
                details_name = %details.name,
                "srrdb details answered for a different release"
            );
            return Ok(None);
        }

        let mut matches = details.archived_files.iter().filter(|member| {
            member.size == size_bytes && member.crc.trim().eq_ignore_ascii_case(crc32_hex)
        });
        let Some(member) = matches.next() else {
            tracing::debug!(
                crc32_hex,
                size_bytes,
                release = %release,
                "srrdb release has no extracted member with this crc and size"
            );
            return Ok(None);
        };
        if matches.next().is_some() {
            tracing::debug!(
                crc32_hex,
                size_bytes,
                release = %release,
                "srrdb release has more than one extracted member with this crc and size"
            );
            return Ok(None);
        }

        let name = member.name.trim();
        if !recovered_name_is_acceptable(name) {
            tracing::debug!(
                crc32_hex,
                release = %release,
                "srrdb recovered name is not an acceptable video file name"
            );
            return Ok(None);
        }

        Ok(Some(name.to_string()))
    }
}

#[cfg(test)]
#[path = "srrdb_tests.rs"]
mod srrdb_tests;
