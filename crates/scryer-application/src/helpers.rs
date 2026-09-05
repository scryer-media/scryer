use super::*;
use std::future::Future;

pub(crate) const INHERIT_QUALITY_PROFILE_VALUE: &str = "__inherit__";
pub(crate) const NATIVE_DOWNLOAD_CLIENT_TYPES: [&str; 3] = ["nzbget", "sabnzbd", "weaver"];

pub(crate) fn parsed_episode_lookup_season(
    ep_meta: &ParsedEpisodeMetadata,
    default_season: &str,
) -> String {
    if ep_meta.season == Some(0) {
        "0".to_string()
    } else {
        default_season.to_string()
    }
}

/// Return the accepted input kinds for a download client type, checking
/// the plugin provider first (WASM plugins), then falling back to known
/// native client capabilities.
///
/// An empty vec means the client has not declared any capabilities and
/// will not receive any downloads.
pub fn accepted_inputs_for_client(
    client_type: &str,
    plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
) -> Vec<DownloadSourceKind> {
    if let Some(provider) = plugin_provider {
        let inputs = provider.accepted_inputs_for_provider(client_type);
        if !inputs.is_empty() {
            return inputs
                .iter()
                .filter_map(|s| DownloadSourceKind::parse(s))
                .collect();
        }
    }
    native_accepted_inputs(client_type)
}

/// Native client capabilities. Returns the accepted input kinds for
/// built-in download client types.
fn native_accepted_inputs(client_type: &str) -> Vec<DownloadSourceKind> {
    match client_type {
        "nzbget" | "sabnzbd" | "weaver" => vec![DownloadSourceKind::NzbFile],
        _ => vec![],
    }
}

/// Lower the calling thread's scheduling priority.
///
/// Call this at the top of CPU-heavy `spawn_blocking` closures (AVIF encoding,
/// alass alignment, audio decoding) so they don't starve the async runtime.
/// Safe to call on supported platforms; silently ignored on Windows.
#[cfg(target_os = "macos")]
pub fn nice_thread() {
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn nice_thread() {
    unsafe {
        libc::nice(10);
    }
}

#[cfg(not(unix))]
pub fn nice_thread() {}

pub(crate) fn normalize_release_attempt_hint(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty())?;
    let Ok(mut url) = url::Url::parse(raw) else {
        return Some(raw.to_string());
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    let mut query = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.to_ascii_lowercase().replace(['_', '-'], "").as_str(),
                "apikey" | "apiaccess" | "token" | "auth" | "password" | "passkey"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    query.sort();
    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Some(url.to_string())
}

/// The canonical form of a release name: trimmed and ASCII-lowercased.
///
/// This is the blocklist's matcher and its unique key, so the writer and every
/// reader must agree on it exactly. It is deliberately ASCII-only and
/// deliberately never expressed in SQL: SQLite's `LOWER` is ASCII-only while
/// Postgres' `lower()` is locale-aware, so a normalization computed in the
/// database would differ between the two engines on a non-ASCII name.
pub fn normalize_release_name(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReleasePasswordClassification {
    Real(String),
    ProtectedFlag,
    UnprotectedFlag,
    Empty,
}

pub(crate) fn classify_release_password(raw: Option<&str>) -> ReleasePasswordClassification {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return ReleasePasswordClassification::Empty;
    };

    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "passworded" | "protected" => {
            ReleasePasswordClassification::ProtectedFlag
        }
        "0" | "false" | "no" => ReleasePasswordClassification::UnprotectedFlag,
        _ => ReleasePasswordClassification::Real(value.to_string()),
    }
}

pub fn normalize_release_password(raw: Option<&str>) -> Option<String> {
    match classify_release_password(raw) {
        ReleasePasswordClassification::Real(value) => Some(value),
        ReleasePasswordClassification::ProtectedFlag
        | ReleasePasswordClassification::UnprotectedFlag
        | ReleasePasswordClassification::Empty => None,
    }
}

pub(crate) fn is_obfuscated_release_name(parsed: &ParsedReleaseMetadata) -> bool {
    if parsed
        .release_group
        .as_ref()
        .is_some_and(|group| !group.trim().is_empty())
    {
        return false;
    }

    parsed
        .raw_title
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 8)
        .any(|token| {
            let has_alpha = token.chars().any(|ch| ch.is_ascii_alphabetic());
            let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
            let hex_like = token.chars().all(|ch| ch.is_ascii_hexdigit());
            (has_alpha && has_digit) || hex_like
        })
}

pub(crate) fn has_usable_release_title_signal(parsed: &ParsedReleaseMetadata) -> bool {
    let normalized_title = parsed.normalized_title.trim();
    if normalized_title.is_empty() {
        return false;
    }

    if matches!(
        normalized_title.to_ascii_uppercase().as_str(),
        "MOVIE" | "VIDEO" | "FILE" | "DOWNLOAD" | "UNKNOWN"
    ) {
        return false;
    }

    !is_obfuscated_release_name(parsed)
}

pub(crate) fn normalize_release_title_signal(
    mut parsed: ParsedReleaseMetadata,
) -> ParsedReleaseMetadata {
    parsed.normalized_title = normalize_compact_part_token(&parsed.normalized_title);
    parsed.normalized_title_variants = parsed
        .normalized_title_variants
        .into_iter()
        .map(|variant| normalize_compact_part_token(&variant))
        .collect();
    parsed
}

pub(crate) fn parse_usable_release_title(raw: &str) -> Option<ParsedReleaseMetadata> {
    let parsed = normalize_release_title_signal(parse_release_metadata(raw));
    has_usable_release_title_signal(&parsed).then_some(parsed)
}

fn normalize_compact_part_token(title: &str) -> String {
    let mut tokens = Vec::new();
    let mut changed = false;

    for token in title.split_whitespace() {
        let upper = token.to_ascii_uppercase();
        if let Some(number) = upper.strip_prefix("PART")
            && !number.is_empty()
            && number.chars().all(|ch| ch.is_ascii_digit())
        {
            tokens.push("PART".to_string());
            tokens.push(number.to_string());
            changed = true;
        } else {
            tokens.push(token.to_string());
        }
    }

    if changed {
        tokens.join(" ")
    } else {
        title.to_string()
    }
}

pub(crate) fn normalize_release_selection_signature(
    source_hint: Option<&str>,
    source_title: Option<&str>,
    source_kind: Option<DownloadSourceKind>,
) -> Option<String> {
    let source_hint = normalize_release_attempt_hint(source_hint);
    let source_title = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let source_kind = source_kind.map(|value| value.as_str().to_string());

    if source_hint.is_none() && source_title.is_none() && source_kind.is_none() {
        return None;
    }

    let identity = format!(
        "v1\0{}\0{}\0{}",
        source_kind.unwrap_or_default(),
        source_hint.unwrap_or_default(),
        source_title.unwrap_or_default()
    );
    // `blake3:v2:` supersedes the former `sha256:v1:`. The value is write-only
    // in production — the only reader, `find_by_title_and_request_signature`,
    // has no application-layer caller — so historical rows keep their old
    // prefix as inert data and need no backfill. The prefix stays so a future
    // reader can tell the two apart rather than silently mismatching.
    Some(format!(
        "blake3:v2:{}",
        blake3_identity_hex(HashDomain::ReleaseSelection, identity)
    ))
}

/// Namespace for a BLAKE3 identity digest.
///
/// Every identity hash in the application is domain-separated, so two digests
/// computed over identical input in different namespaces can never collide or
/// be substituted for one another. Variants are never renamed or reused: the
/// string is baked into persisted values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashDomain {
    /// Convergence scope search-criteria fingerprint.
    ConvergenceScope,
    /// Per-indexer coverage fingerprint.
    IndexerCoverage,
    /// Quality-profile acceptance-criteria version.
    QualityProfileCriteria,
    /// Canonical episode-set convergence scope key.
    EpisodeSetScope,
    /// Canonical series-pack set convergence scope key.
    SeriesPackSetScope,
    /// Release-selection / release-attempt signature.
    ReleaseSelection,
    /// Library probe signature (directory and file schemes).
    LibraryProbe,
    /// Media-request dedup identity.
    MediaRequestIdentity,
    /// Library-scan unmatched item row id.
    LibraryScanUnmatchedItem,
    /// Authorization + session claim fingerprint.
    AuthorizationFingerprint,
    /// User-delete preview confirmation fingerprint.
    DeletePreview,
    /// Rename-plan content hash.
    RenamePlan,
    /// Location-operation plan fingerprint (FR-081).
    LocationPlan,
    /// Indexer credential fingerprint inside the search identity.
    IndexerSecret,
    /// Search-diagnostics query signature.
    IndexerQuerySignature,
    /// Search-diagnostics indexer identity fingerprint (reuse validity).
    IndexerSearchIdentity,
    /// Interactive-search candidate identity within one session.
    CandidateSessionIdentity,
    /// Serialized request-rule input document (spec 0003 FR-016).
    RequestRuleInput,
}

impl HashDomain {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ConvergenceScope => "scryer.convergence.scope.v1",
            Self::IndexerCoverage => "scryer.convergence.indexer.v1",
            Self::QualityProfileCriteria => "scryer.quality.profile-criteria.v1",
            Self::EpisodeSetScope => "scryer.convergence.episode-set.v1",
            Self::SeriesPackSetScope => "scryer.convergence.series-pack-set.v1",
            Self::ReleaseSelection => "scryer.release.selection.v1",
            Self::LibraryProbe => "scryer.library.probe.v1",
            Self::MediaRequestIdentity => "scryer.media-request.identity.v1",
            Self::LibraryScanUnmatchedItem => "scryer.library.scan-unmatched.v1",
            Self::AuthorizationFingerprint => "scryer.auth.fingerprint.v1",
            Self::DeletePreview => "scryer.library.delete-preview.v1",
            Self::RenamePlan => "scryer.library.rename-plan.v1",
            Self::LocationPlan => "scryer.location.plan.v1",
            Self::IndexerSecret => "scryer.indexer.secret.v1",
            Self::IndexerQuerySignature => "scryer.indexer.query-signature.v1",
            Self::IndexerSearchIdentity => "scryer.indexer.search-identity.v1",
            Self::CandidateSessionIdentity => "scryer.indexer.candidate-session.v1",
            Self::RequestRuleInput => "scryer.request-rule.input.v1",
        }
    }
}

/// Domain-separated BLAKE3 identity digest, lowercase hex.
///
/// The domain label is absorbed first, followed by a NUL byte that cannot
/// appear in a label, so no input can impersonate a different domain.
///
/// # This is the only hash first-party code may use
///
/// Every identity, fingerprint, signature, dedup key, cache key, and content
/// digest Scryer computes for its own purposes goes through here. Add a
/// [`HashDomain`] variant rather than reaching for another algorithm.
///
/// There is deliberately no `sha256_hex` beside it. SHA-256 survives in this
/// codebase **only** where an external contract fixes the algorithm and we have
/// no choice:
///
/// - WebAuthn `rpIdHash` and ECDSA-P256-SHA256 verification (`scryer-webauthn`)
/// - TOTP, where the algorithm is user-selectable per RFC 6238 and stored
///   (`security/totp.rs`)
/// - HMAC-SHA256 signing for JWTs, sessions, API keys, and OAuth
/// - OAuth PKCE `S256` challenges, fixed by RFC 7636 (`oauth.rs`)
/// - GraphQL Automatic Persisted Queries, where the server keys on SHA-256 of
///   the query text (`scryer-infrastructure-metadata`)
/// - Sigstore/Rekor `hashedrekord` verification, which recomputes a digest the
///   transparency log already recorded (`plugins/catalog.rs`)
/// - SHA-384 migration-asset integrity (`scryer-infrastructure-datastore`)
/// - The trusted-certificate `fingerprint_sha256` a human compares against what
///   a browser or `openssl` prints (`settings/runtime/general.rs`)
///
/// Those are compatibility surfaces, not a precedent. If a new hash is *ours* —
/// nobody outside Scryer computes or verifies it — it belongs here. If you think
/// you need SHA-256 for something first-party, the answer is that you do not.
pub fn blake3_identity_hex(domain: HashDomain, input: impl AsRef<str>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_str().as_bytes());
    hasher.update(&[0u8]);
    hasher.update(input.as_ref().as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn to_hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

/// Total and available bytes of the filesystem backing a path.
///
/// `available_bytes` is the space an unprivileged writer can use (the
/// `f_bavail` figure on unix, the caller-available figure on Windows), so
/// `total - available` counts reserved blocks as used, matching what an
/// operator expects a usage percentage to mean.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FilesystemSpace {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Widen a `statvfs` counter to `u64`.
///
/// The block-count fields are `u32` on some unix targets and `u64` on others,
/// so callers cannot write `u64::from(..)` without tripping
/// `clippy::useless_conversion` on whichever platform already matches.
#[cfg(all(unix, not(target_os = "macos")))]
fn statvfs_field_to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

#[cfg(unix)]
fn unix_c_path(path: &std::path::Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("disk-space path contains an interior NUL: {error}"),
        )
    })
}

/// `statfs`, not `statvfs`: Darwin's `statvfs` interface carries 32-bit block
/// counts (`fsblkcnt_t` is `unsigned int`), so any volume larger than
/// `2^32 * f_frsize` bytes wraps — a 23 TB SMB share reads back as ~1 TB.
/// Darwin's `statfs` carries 64-bit counts sized in `f_bsize` units.
#[cfg(target_os = "macos")]
pub(crate) fn filesystem_space_raw(path: &std::path::Path) -> std::io::Result<FilesystemSpace> {
    let c_path = unix_c_path(path)?;
    unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let bsize = u64::from(buf.f_bsize);
        Ok(FilesystemSpace {
            total_bytes: buf.f_blocks.saturating_mul(bsize),
            available_bytes: buf.f_bavail.saturating_mul(bsize),
        })
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn filesystem_space_raw(path: &std::path::Path) -> std::io::Result<FilesystemSpace> {
    let c_path = unix_c_path(path)?;
    unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut buf) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Some FUSE filesystems report only `f_bsize`; POSIX says counts are
        // in `f_frsize` units, so prefer it but fall back when it is zero.
        let mut frsize = statvfs_field_to_u64(buf.f_frsize);
        if frsize == 0 {
            frsize = statvfs_field_to_u64(buf.f_bsize);
        }
        Ok(FilesystemSpace {
            total_bytes: statvfs_field_to_u64(buf.f_blocks).saturating_mul(frsize),
            available_bytes: statvfs_field_to_u64(buf.f_bavail).saturating_mul(frsize),
        })
    }
}

#[cfg(windows)]
pub(crate) fn filesystem_space_raw(path: &std::path::Path) -> std::io::Result<FilesystemSpace> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let mut available = 0_u64;
    let mut total = 0_u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut available,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FilesystemSpace {
        total_bytes: total,
        available_bytes: available,
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn filesystem_space_raw(_path: &std::path::Path) -> std::io::Result<FilesystemSpace> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "disk-space queries are unsupported on this platform",
    ))
}

/// Filesystem usage for `path`, or `None` when it cannot be inspected or the
/// filesystem reports a degenerate zero total (some FUSE and network mounts).
pub(crate) fn filesystem_space(path: &str) -> Option<FilesystemSpace> {
    filesystem_space_raw(std::path::Path::new(path))
        .ok()
        .filter(|space| space.total_bytes > 0)
}

fn normalize_tag(raw: String) -> String {
    raw.trim().to_lowercase()
}

fn normalize_show_text(raw: String) -> Option<String> {
    let value = raw.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

pub(crate) fn normalize_show_text_opt(raw: Option<String>) -> Option<String> {
    raw.and_then(normalize_show_text)
}

pub(crate) fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in raw {
        let normalized = normalize_tag(value.clone());
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

pub(crate) fn sanitize_ids(ids: Vec<ExternalId>) -> Vec<ExternalId> {
    ids.into_iter()
        .filter_map(|id| {
            let source = id.source.trim().to_lowercase();
            let value = id.value.trim().to_string();
            if source.is_empty() || value.is_empty() {
                None
            } else {
                Some(ExternalId { source, value })
            }
        })
        .collect()
}

pub(crate) async fn await_cancellable<T, F>(
    cancel_token: Option<&tokio_util::sync::CancellationToken>,
    future: F,
) -> Option<T>
where
    F: Future<Output = T>,
{
    let Some(token) = cancel_token else {
        return Some(future.await);
    };

    tokio::pin!(future);
    tokio::select! {
        _ = token.cancelled() => None,
        value = &mut future => Some(value),
    }
}

pub(crate) async fn await_cancellable_app_result<T, F>(
    cancel_token: Option<&tokio_util::sync::CancellationToken>,
    future: F,
) -> AppResult<Option<T>>
where
    F: Future<Output = AppResult<T>>,
{
    match await_cancellable(cancel_token, future).await {
        Some(result) => result.map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_selection_signature_is_credential_free_and_versioned() {
        let signature = |key: &str, id: u8| {
            normalize_release_selection_signature(
                Some(&format!(
                    "https://indexer.invalid/api?t=get&id={id}&apikey={key}"
                )),
                Some("Synthetic.Release.1080p"),
                Some(DownloadSourceKind::NzbUrl),
            )
            .expect("signature")
        };

        assert_eq!(signature("first-secret", 1), signature("rotated-secret", 1));
        assert_ne!(signature("first-secret", 1), signature("first-secret", 2));
        assert!(signature("first-secret", 1).starts_with("blake3:v2:"));
        assert!(!signature("first-secret", 1).contains("first-secret"));
    }

    struct StubDownloadClientPluginProvider {
        available_types: Vec<String>,
        accepted_inputs: Vec<String>,
    }

    impl DownloadClientPluginProvider for StubDownloadClientPluginProvider {
        fn client_for_config(
            &self,
            _config: &DownloadClientConfig,
        ) -> Option<Arc<dyn DownloadClient>> {
            None
        }

        fn available_provider_types(&self) -> Vec<String> {
            self.available_types.clone()
        }

        fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
            if self
                .available_types
                .iter()
                .any(|value| value.eq_ignore_ascii_case(provider_type))
            {
                return self.accepted_inputs.clone();
            }
            Vec::new()
        }
    }

    #[test]
    fn accepted_inputs_for_client_does_not_advertise_qbittorrent_without_plugin() {
        let inputs = accepted_inputs_for_client("qbittorrent", None);
        assert!(inputs.is_empty());
    }

    #[test]
    fn accepted_inputs_for_client_uses_qbittorrent_plugin_capabilities_when_available() {
        let provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(StubDownloadClientPluginProvider {
                available_types: vec!["qbittorrent".to_string()],
                accepted_inputs: vec!["magnet_uri".to_string(), "torrent_file".to_string()],
            });

        let inputs = accepted_inputs_for_client("qbittorrent", Some(&provider));
        assert_eq!(
            inputs,
            vec![
                DownloadSourceKind::MagnetUri,
                DownloadSourceKind::TorrentFile
            ]
        );
    }
}
