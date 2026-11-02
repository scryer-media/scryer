//! Provider adapters for media-server watch signals (RFC 137 section 7.3).
//!
//! One [`MediaServerSignalSource`] implementation that dispatches on the
//! connection's provider: Jellyfin, Emby, and Plex. Every arm answers with the
//! participant's complete played set or an error. A partial read is never an
//! answer, because "this read stopped early" and "this person has watched
//! nothing" must never be the same answer.
//!
//! # Jellyfin and Emby
//!
//! The two servers share the item shape, so one reader serves both and only the
//! credential differs: Jellyfin's `Authorization: MediaBrowser` header (with
//! the legacy header beside it), Emby's `X-Emby-Token` with its client header.
//!
//! `GET /Users/{userId}/Items` with `Filters=IsPlayed`, recursive over the
//! whole library, restricted to `Movie,Episode`, and asking for the fields the
//! mapper needs: `ProviderIds` for the item, `SeriesId`/`SeriesName` and the
//! season/episode indexes for episodes, and `UserData` for played state, play
//! count, last-played time, and the favourite flag. Paged with
//! `StartIndex`/`Limit`. Keys are read case-insensitively, so a server that
//! serializes `userData` rather than `UserData` is still read.
//!
//! Two bounds, both deliberate:
//!
//! * a **page size** so a large library does not arrive as one enormous body;
//! * a **hard cap** on total items, after which an incomplete read fails so
//!   the previous watch history and its freshness timestamp are preserved.
//!
//! # Plex
//!
//! Plex keeps watch state per account, and the stored credential is the server
//! owner's token. Server history names the owner as account `1` and every
//! shared user by their plex.tv account id, so the owner's plex.tv id is read
//! first (`users/account.json`, the same call that verified the link) to tell
//! the two apart:
//!
//! * **the owner** is read from the library itself: each movie and show
//!   section's `/library/sections/{key}/all` with `type=1` (movies) or `type=4`
//!   (episodes), `viewCount>>=0` (Plex's "greater than"), and `includeGuids=1`.
//!   `viewCount` and `lastViewedAt` are the owner's own.
//! * **a shared user** cannot be read through the library with the owner's
//!   token, so their plays come from `/status/sessions/history/all` filtered by
//!   `accountID`. Plays are folded per `ratingKey` into a count and the newest
//!   `viewedAt`, then the items are looked up in batches through
//!   `/library/metadata/{keys}` for their `Guid` list and episode coordinates.
//!   A history entry whose item has left the library has nothing to map and is
//!   dropped.
//!
//! Both reads page with the `X-Plex-Container-Start` and
//! `X-Plex-Container-Size` request headers. Plex has no favourite, so Plex
//! items always report `is_favorite = false`. No user password is ever stored
//! or sent.
//!
//! # Series ids
//!
//! None of the three servers puts the series' TMDB/TVDB ids on the episode.
//! Rather than one lookup per episode, the adapter collects the distinct series
//! it saw and resolves them in batches, then attaches each series' external ids
//! to its episodes. A series that cannot be resolved leaves its episodes
//! without series ids, which mapping treats as unmapped rather than guessing.
//!
//! # Malformed items
//!
//! An item without an id, or a played-set item with a type that is neither a
//! movie nor an episode, makes the read incomplete. Publishing a partial
//! history could turn an omitted played item into a false "never watched"
//! maintenance fact.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use scryer_application::{AppError, AppResult, MediaServerSignalSource, ProviderPlayedItem};
use scryer_domain::{MediaServerConnection, MediaServerProvider, MediaServerSignalKind};
use scryer_outbound_http::generic_reqwest_client;
use serde_json::Value;
use tracing::warn;
use url::Url;

use crate::jellyfin::JellyfinAuth;

/// Per-request budget. Higher than the playback probe's five seconds: this is a
/// paged library query, not a memory read, but it still sits inside a
/// background job that must not hang for minutes.
const SIGNAL_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Items requested per page, for every provider.
pub const SIGNAL_PAGE_SIZE: usize = 500;

/// Hard ceiling on items read for one participant in one sweep. Beyond this the
/// read fails as incomplete rather than publishing a truncated history.
pub const SIGNAL_MAX_ITEMS: usize = 20_000;

/// Series and item metadata lookups are batched; this bounds one batch's id
/// list so the request line stays a sane length.
const LOOKUP_BATCH: usize = 100;

/// The `Fields` list is deliberately short: it may name only real members of
/// Jellyfin's `ItemFields` enum, and a value that is not one is not "extra data"
/// — some server versions reject the whole request over it.
///
/// Everything else this adapter reads off an item row (`Id`, `Type`, `SeriesId`,
/// `SeriesName`, `IndexNumber`, `ParentIndexNumber`) is an unconditional
/// `BaseItemDto` property that arrives whether or not it is requested, and the
/// user-data block is governed by `EnableUserData=true` rather than by a field
/// name. So the list carries only the three that genuinely have to be asked for.
const JELLYFIN_ITEM_FIELDS: &str = "ProviderIds,ParentId,Path";

/// Emby's list: only the provider ids need asking for. The user-data block is
/// switched on by `EnableUserData=true`, as on Jellyfin.
const EMBY_ITEM_FIELDS: &str = "ProviderIds";

/// External id sources Scryer joins on, mapped from Jellyfin's and Emby's
/// `ProviderIds` keys. The capitalization has drifted across versions, so the
/// match is case-insensitive on the key rather than a fixed spelling.
const PROVIDER_ID_KEYS: [(&str, &str); 3] = [("tmdb", "tmdb"), ("tvdb", "tvdb"), ("imdb", "imdb")];

/// Plex's account service, where the owner's account id is read.
const PLEX_SERVICE_URL: &str = "https://plex.tv/";

/// A Plex connection whose base URL is still the account service has no
/// selected server, so there is no library or history to read.
const PLEX_SERVICE_HOST: &str = "plex.tv";

/// Plex library type numbers for the two kinds this adapter records.
const PLEX_TYPE_MOVIE: &str = "1";
const PLEX_TYPE_EPISODE: &str = "4";

/// The credential and header family one request carries.
#[derive(Clone, Copy)]
enum SignalAuth<'a> {
    Jellyfin(&'a str),
    Emby {
        token: &'a str,
        connection_id: &'a str,
    },
    Plex(&'a str),
}

impl SignalAuth<'_> {
    fn apply(self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Jellyfin(token) => request.jellyfin_auth(token, None),
            Self::Emby {
                token,
                connection_id,
            } => request.header("X-Emby-Token", token).header(
                "X-Emby-Authorization",
                crate::emby::client_authorization(connection_id, None),
            ),
            Self::Plex(token) => request.header("X-Plex-Token", token),
        }
    }
}

/// One Plex page request: where to start and how many to take, sent as the
/// container headers rather than query parameters.
#[derive(Clone, Copy)]
struct PlexPage {
    start: usize,
    size: usize,
}

pub struct HttpMediaServerSignalSource {
    client: reqwest::Client,
    plex_service_url: Url,
}

impl Default for HttpMediaServerSignalSource {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMediaServerSignalSource {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_service_url: Url::parse(PLEX_SERVICE_URL).expect("valid Plex service URL"),
        }
    }

    #[cfg(test)]
    fn with_plex_service_url(plex_service_url: Url) -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_service_url,
        }
    }

    /// One authenticated read, with the provider's own credential header.
    async fn fetch_json(
        &self,
        url: Url,
        auth: SignalAuth<'_>,
        page: Option<PlexPage>,
    ) -> AppResult<Value> {
        self.request_json(url, auth, page)
            .await
            .map_err(FetchFailure::into_app_error)
    }

    /// [`Self::fetch_json`] with the failure still typed, for the one caller
    /// that treats "not found" differently from every other refusal.
    async fn request_json(
        &self,
        url: Url,
        auth: SignalAuth<'_>,
        page: Option<PlexPage>,
    ) -> Result<Value, FetchFailure> {
        let mut request = auth
            .apply(self.client.get(url))
            .header("Accept", "application/json")
            .timeout(SIGNAL_FETCH_TIMEOUT);
        if let Some(page) = page {
            request = request
                .header("X-Plex-Container-Start", page.start.to_string())
                .header("X-Plex-Container-Size", page.size.to_string());
        }
        let response = request
            .send()
            .await
            .map_err(|error| FetchFailure::Transport(transport_reason(&error)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchFailure::Status(status));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| FetchFailure::Unreadable)
    }

    // ── Jellyfin and Emby ───────────────────────────────────────────────────

    /// Every played movie and episode for one Jellyfin or Emby user.
    async fn fetch_user_items(
        &self,
        connection: &MediaServerConnection,
        auth: SignalAuth<'_>,
        fields: &str,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let provider = connection.provider.as_str();
        let base = server_base(&connection.base_url)?;
        let mut items: Vec<ProviderPlayedItem> = Vec::new();
        let mut skipped = 0_usize;
        let mut start_index = 0_usize;

        loop {
            let url = played_items_url(&base, external_user_id, start_index, fields)?;
            let body = self.fetch_json(url, auth, None).await?;
            let page = field(&body, "Items")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| AppError::Repository("unreadable played history".into()))?;
            if page.is_empty() {
                break;
            }

            let (mut normalized, page_skipped) = normalize_jellyfin_items(&page);
            skipped += page_skipped;
            items.append(&mut normalized);

            start_index += page.len();
            if page.len() < SIGNAL_PAGE_SIZE {
                break;
            }
            if start_index >= SIGNAL_MAX_ITEMS {
                warn!(
                    connection_id = connection.id.as_str(),
                    provider,
                    cap = SIGNAL_MAX_ITEMS,
                    "played history exceeded the per-participant cap; the read was truncated"
                );
                return Err(AppError::Repository(
                    "incomplete played history: item cap reached".into(),
                ));
            }
        }

        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                provider, skipped, "skipped unreadable played items"
            );
            return Err(AppError::Repository(
                "incomplete played history: unreadable items".into(),
            ));
        }

        self.attach_series_ids(&base, auth, external_user_id, &mut items)
            .await;
        Ok(items)
    }

    /// Resolve the external ids of every distinct series the episodes belong
    /// to, in batches, and attach them.
    ///
    /// Failure here is not fatal: episodes simply stay unmapped, which is the
    /// honest outcome. That is why this returns `()` rather than a result.
    async fn attach_series_ids(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        external_user_id: &str,
        items: &mut [ProviderPlayedItem],
    ) {
        let series_ids = distinct_series_ids(items);
        if series_ids.is_empty() {
            return;
        }

        let mut resolved: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for chunk in series_ids.chunks(LOOKUP_BATCH) {
            let url = match series_lookup_url(base, external_user_id, chunk) {
                Ok(url) => url,
                Err(error) => {
                    warn!(error = %error, "could not build the series lookup URL");
                    return;
                }
            };
            match self.fetch_json(url, auth, None).await {
                Ok(body) => {
                    let mut matched = 0_usize;
                    for entry in field(&body, "Items")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                    {
                        let Some(id) = string_field(&entry, "Id") else {
                            continue;
                        };
                        if chunk.contains(&id) {
                            matched += 1;
                        }
                        resolved.insert(id, external_ids(&entry));
                    }
                    // A 200 with nothing usable in it is indistinguishable from
                    // success at every later step: the episodes simply stay
                    // unmapped, and the sweep reports a clean read. It is also
                    // exactly what a wrong query shape produces, so say so.
                    if matched == 0 {
                        warn!(
                            unresolved = chunk.len(),
                            "a series lookup batch resolved none of the requested series ids; those episodes stay unmapped"
                        );
                    }
                }
                Err(error) => {
                    warn!(error = %error, "could not resolve series ids; those episodes stay unmapped");
                    return;
                }
            }
        }

        apply_series_ids(items, &resolved);
    }

    // ── Plex ────────────────────────────────────────────────────────────────

    /// Every played movie and episode for one Plex account on this server.
    async fn fetch_plex(
        &self,
        connection: &MediaServerConnection,
        token: &str,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let base = server_base(&connection.base_url)?;
        if base
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(PLEX_SERVICE_HOST))
        {
            return Err(AppError::Repository("no Plex server selected".into()));
        }
        let auth = SignalAuth::Plex(token);

        let owner_ids = self.plex_owner_account_ids(token).await?;
        let mut items = if owner_ids.iter().any(|id| id == external_user_id) {
            self.fetch_plex_watched_set(connection, &base, auth).await?
        } else {
            // History filters on Plex's numeric account id. Anything else
            // would match nobody and read back as a clean, empty history.
            if !external_user_id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(AppError::Repository(
                    "Plex account id is not a server account id".into(),
                ));
            }
            self.fetch_plex_history(connection, &base, auth, external_user_id)
                .await?
        };

        self.attach_plex_series_ids(&base, auth, &mut items).await;
        Ok(items)
    }

    /// The plex.tv ids of the account the stored token belongs to: `id`, and
    /// `uuid` as the fallback the link verification also accepts.
    async fn plex_owner_account_ids(&self, token: &str) -> AppResult<Vec<String>> {
        let url = self
            .plex_service_url
            .join("users/account.json")
            .map_err(|_| AppError::Repository("invalid Plex service URL".into()))?;
        let body = self.fetch_json(url, SignalAuth::Plex(token), None).await?;
        let user = body
            .get("user")
            .ok_or_else(|| AppError::Repository("unreadable Plex account".into()))?;
        let ids = ["id", "uuid"]
            .into_iter()
            .filter_map(|key| user.get(key).and_then(scalar_string))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(AppError::Repository("unreadable Plex account".into()));
        }
        Ok(ids)
    }

    /// The owner's played set, straight from the movie and show libraries.
    async fn fetch_plex_watched_set(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        auth: SignalAuth<'_>,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let sections = self
            .fetch_json(plex_url(base, "library/sections")?, auth, None)
            .await?;
        let directories = plex_container(&sections)?
            .get("Directory")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut items = Vec::new();
        let mut skipped = 0_usize;
        let mut read = 0_usize;
        for directory in directories {
            let plex_type = match directory.get("type").and_then(Value::as_str) {
                Some("movie") => PLEX_TYPE_MOVIE,
                Some("show") => PLEX_TYPE_EPISODE,
                _ => continue,
            };
            let Some(key) = directory.get("key").and_then(scalar_string) else {
                // A movie or show library Scryer cannot address is a library
                // whose plays would silently go missing.
                return Err(AppError::Repository(
                    "incomplete played history: unreadable library".into(),
                ));
            };
            let mut url = plex_url(base, &format!("library/sections/{key}/all"))?;
            url.query_pairs_mut()
                .append_pair("type", plex_type)
                .append_pair("viewCount>>", "0")
                .append_pair("includeGuids", "1");
            let entries = self.fetch_plex_entries(connection, url, auth, read).await?;
            read += entries.len();
            for entry in &entries {
                match normalize_plex_item(entry) {
                    Some(item) => items.push(item),
                    None => skipped += 1,
                }
            }
        }

        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped, "skipped unreadable Plex played items"
            );
            return Err(AppError::Repository(
                "incomplete played history: unreadable items".into(),
            ));
        }
        Ok(items)
    }

    /// A shared user's plays, from server history filtered to their account.
    async fn fetch_plex_history(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        auth: SignalAuth<'_>,
        account_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let mut url = plex_url(base, "status/sessions/history/all")?;
        url.query_pairs_mut()
            .append_pair("accountID", account_id)
            .append_pair("sort", "viewedAt:desc");
        let entries = self.fetch_plex_entries(connection, url, auth, 0).await?;

        let (plays, skipped) = fold_plex_history(&entries);
        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped, "skipped unreadable Plex history entries"
            );
            return Err(AppError::Repository(
                "incomplete played history: unreadable items".into(),
            ));
        }

        let keys = plays.keys().cloned().collect::<Vec<_>>();
        let mut items = Vec::with_capacity(plays.len());
        let mut unreadable = 0_usize;
        for chunk in keys.chunks(LOOKUP_BATCH) {
            for entry in self.plex_metadata(base, auth, chunk).await? {
                let Some(mut item) = normalize_plex_item(&entry) else {
                    unreadable += 1;
                    continue;
                };
                let Some(play) = plays.get(&item.provider_item_id) else {
                    continue;
                };
                // The metadata was read with the owner's token, so its own
                // view count and last-viewed time are the owner's. This
                // account's come from its history alone.
                item.played = true;
                item.play_count = play.count;
                item.last_played_at = play.last_viewed_at;
                items.push(item);
            }
        }
        if unreadable > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped = unreadable,
                "skipped unreadable Plex items in history"
            );
            return Err(AppError::Repository(
                "incomplete played history: unreadable items".into(),
            ));
        }
        let gone = plays.len().saturating_sub(items.len());
        if gone > 0 {
            // Played once, since removed from the library: there is nothing
            // left on the server for the play to be about.
            warn!(
                connection_id = connection.id.as_str(),
                gone, "Plex history names items no longer in the library; they are not recorded"
            );
        }
        Ok(items)
    }

    /// Every `Metadata` entry of a paged Plex listing, failing as incomplete
    /// when the listing cannot be read whole. `already_read` counts what the
    /// participant's earlier listings returned, so the cap is per participant.
    async fn fetch_plex_entries(
        &self,
        connection: &MediaServerConnection,
        url: Url,
        auth: SignalAuth<'_>,
        already_read: usize,
    ) -> AppResult<Vec<Value>> {
        let mut entries = Vec::new();
        let mut start = 0_usize;
        loop {
            let page = PlexPage {
                start,
                size: SIGNAL_PAGE_SIZE,
            };
            let body = self.fetch_json(url.clone(), auth, Some(page)).await?;
            let container = plex_container(&body)?;
            // A server that answers a later page from the top again would
            // otherwise loop or duplicate; neither is a complete read.
            if container
                .get("offset")
                .and_then(integer_field)
                .is_some_and(|offset| offset != start as i64)
            {
                return Err(AppError::Repository(
                    "incomplete played history: paging was not honoured".into(),
                ));
            }
            let page_entries = match container.get("Metadata") {
                None => Vec::new(),
                Some(value) => value
                    .as_array()
                    .cloned()
                    .ok_or_else(|| AppError::Repository("unreadable played history".into()))?,
            };
            let page_len = page_entries.len();
            entries.extend(page_entries);
            start += page_len;
            let total = container.get("totalSize").and_then(integer_field);

            if already_read + entries.len() >= SIGNAL_MAX_ITEMS {
                warn!(
                    connection_id = connection.id.as_str(),
                    cap = SIGNAL_MAX_ITEMS,
                    "Plex played history exceeded the per-participant cap; the read was truncated"
                );
                return Err(AppError::Repository(
                    "incomplete played history: item cap reached".into(),
                ));
            }
            if page_len < SIGNAL_PAGE_SIZE || total.is_some_and(|total| start as i64 >= total) {
                return Ok(entries);
            }
        }
    }

    /// Metadata rows for a batch of rating keys. A batch the server refuses as
    /// not found is retried one key at a time, so one item that has left the
    /// library does not hide the rest; a key that is itself not found is
    /// simply absent from the answer.
    async fn plex_metadata(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        keys: &[String],
    ) -> AppResult<Vec<Value>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        match self
            .request_json(plex_metadata_url(base, keys)?, auth, None)
            .await
        {
            Ok(body) => plex_metadata_entries(&body),
            Err(FetchFailure::Status(StatusCode::NOT_FOUND)) if keys.len() > 1 => {
                let mut entries = Vec::new();
                for key in keys {
                    let url = plex_metadata_url(base, std::slice::from_ref(key))?;
                    match self.request_json(url, auth, None).await {
                        Ok(body) => entries.extend(plex_metadata_entries(&body)?),
                        Err(FetchFailure::Status(StatusCode::NOT_FOUND)) => {}
                        Err(failure) => return Err(failure.into_app_error()),
                    }
                }
                Ok(entries)
            }
            Err(FetchFailure::Status(StatusCode::NOT_FOUND)) => Ok(Vec::new()),
            Err(failure) => Err(failure.into_app_error()),
        }
    }

    /// Plex's series ids, from the shows' own metadata. Not fatal, exactly as
    /// for Jellyfin and Emby.
    async fn attach_plex_series_ids(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        items: &mut [ProviderPlayedItem],
    ) {
        let series_ids = distinct_series_ids(items);
        if series_ids.is_empty() {
            return;
        }
        let mut resolved: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for chunk in series_ids.chunks(LOOKUP_BATCH) {
            match self.plex_metadata(base, auth, chunk).await {
                Ok(entries) => {
                    for entry in entries {
                        let Some(id) = entry.get("ratingKey").and_then(scalar_string) else {
                            continue;
                        };
                        resolved.insert(id, plex_external_ids(&entry));
                    }
                }
                Err(error) => {
                    warn!(error = %error, "could not resolve Plex series ids; those episodes stay unmapped");
                    return;
                }
            }
        }
        apply_series_ids(items, &resolved);
    }
}

#[async_trait]
impl MediaServerSignalSource for HttpMediaServerSignalSource {
    async fn fetch_played_items(
        &self,
        connection: &MediaServerConnection,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let api_key = connection
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| AppError::Repository("no credential stored".into()))?;
        let external_user_id = external_user_id.trim();
        if external_user_id.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::IDENTITY_MEDIA_SERVER_SIGNAL_USER_ID_REQUIRED,
                "a media-server signal read needs a provider user id",
            ));
        }

        match connection.provider {
            MediaServerProvider::Jellyfin => {
                self.fetch_user_items(
                    connection,
                    SignalAuth::Jellyfin(api_key),
                    JELLYFIN_ITEM_FIELDS,
                    external_user_id,
                )
                .await
            }
            MediaServerProvider::Emby => {
                self.fetch_user_items(
                    connection,
                    SignalAuth::Emby {
                        token: api_key,
                        connection_id: &connection.id,
                    },
                    EMBY_ITEM_FIELDS,
                    external_user_id,
                )
                .await
            }
            MediaServerProvider::Plex => {
                self.fetch_plex(connection, api_key, external_user_id).await
            }
        }
    }
}

// ── Normalization ───────────────────────────────────────────────────────────

/// Turn one page of Jellyfin or Emby items into normalized observations.
///
/// Returns `(items, skipped)`. Pure, so the whole contract is testable against
/// canned payloads without an HTTP server.
pub fn normalize_jellyfin_items(page: &[Value]) -> (Vec<ProviderPlayedItem>, usize) {
    let mut items = Vec::with_capacity(page.len());
    let mut skipped = 0;
    for entry in page {
        match normalize_jellyfin_item(entry) {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    (items, skipped)
}

/// One Jellyfin or Emby item, or `None` when it cannot be read as a played
/// movie or episode.
pub fn normalize_jellyfin_item(entry: &Value) -> Option<ProviderPlayedItem> {
    let provider_item_id = string_field(entry, "Id")?;
    let kind = match string_field(entry, "Type")?.to_ascii_lowercase().as_str() {
        "movie" => MediaServerSignalKind::Movie,
        "episode" => MediaServerSignalKind::Episode,
        _ => return None,
    };

    let user_data = field(entry, "UserData");
    // `Filters=IsPlayed` already narrowed the query, but the row records what
    // the server said rather than what the filter implied.
    let played = user_data
        .and_then(|data| field(data, "Played"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let play_count = user_data
        .and_then(|data| field(data, "PlayCount"))
        .and_then(integer_field)
        .unwrap_or(0)
        .max(0);
    let last_played_at = user_data
        .and_then(|data| field(data, "LastPlayedDate"))
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let is_favorite = user_data
        .and_then(|data| field(data, "IsFavorite"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(ProviderPlayedItem {
        provider_item_id,
        kind,
        name: string_field(entry, "Name"),
        external_ids: external_ids(entry),
        // Filled in by the batched series lookup; an episode whose series never
        // resolves keeps an empty map and stays unmapped.
        series_external_ids: BTreeMap::new(),
        series_provider_item_id: string_field(entry, "SeriesId"),
        // Both servers name the season "ParentIndexNumber" on an episode.
        season_number: field(entry, "ParentIndexNumber").and_then(integer_field),
        episode_number: field(entry, "IndexNumber").and_then(integer_field),
        played,
        play_count,
        last_played_at,
        is_favorite,
    })
}

/// One Plex movie or episode, or `None` when it cannot be read as one.
///
/// Used for the owner's watched set as read, and for history items before
/// their per-account play facts replace the owner's.
pub fn normalize_plex_item(entry: &Value) -> Option<ProviderPlayedItem> {
    let provider_item_id = entry.get("ratingKey").and_then(scalar_string)?;
    let kind = match entry.get("type").and_then(Value::as_str)? {
        "movie" => MediaServerSignalKind::Movie,
        "episode" => MediaServerSignalKind::Episode,
        _ => return None,
    };
    let view_count = entry.get("viewCount").and_then(integer_field);
    let is_episode = kind == MediaServerSignalKind::Episode;

    Some(ProviderPlayedItem {
        provider_item_id,
        kind,
        name: entry
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        external_ids: plex_external_ids(entry),
        series_external_ids: BTreeMap::new(),
        series_provider_item_id: is_episode
            .then(|| entry.get("grandparentRatingKey").and_then(scalar_string))
            .flatten(),
        season_number: is_episode
            .then(|| entry.get("parentIndex").and_then(integer_field))
            .flatten(),
        episode_number: is_episode
            .then(|| entry.get("index").and_then(integer_field))
            .flatten(),
        // The watched-set query asked for `viewCount > 0`; the row still
        // records what the server said.
        played: view_count.is_none_or(|count| count > 0),
        play_count: view_count.unwrap_or(0).max(0),
        last_played_at: entry.get("lastViewedAt").and_then(epoch_timestamp),
        // Plex has no favourite.
        is_favorite: false,
    })
}

/// One account's plays of one item, folded from its history entries.
#[derive(Debug, Default, PartialEq, Eq)]
struct PlexPlays {
    count: i64,
    last_viewed_at: Option<DateTime<Utc>>,
}

/// Fold history entries into plays per rating key. Returns `(plays, skipped)`.
///
/// An entry with no rating key or no type is unreadable. An entry of a type
/// this adapter does not record (music, photos) is not a movie or episode play
/// at all, so it is passed over rather than counted.
fn fold_plex_history(entries: &[Value]) -> (BTreeMap<String, PlexPlays>, usize) {
    let mut plays: BTreeMap<String, PlexPlays> = BTreeMap::new();
    let mut skipped = 0_usize;
    for entry in entries {
        let Some(rating_key) = entry.get("ratingKey").and_then(scalar_string) else {
            skipped += 1;
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("movie" | "episode") => {}
            Some(_) => continue,
            None => {
                skipped += 1;
                continue;
            }
        }
        let viewed_at = entry.get("viewedAt").and_then(epoch_timestamp);
        let play = plays.entry(rating_key).or_default();
        play.count += 1;
        if viewed_at > play.last_viewed_at {
            play.last_viewed_at = viewed_at;
        }
    }
    (plays, skipped)
}

/// TMDB/TVDB/IMDb ids from a Jellyfin or Emby `ProviderIds` object, lowercased
/// to the source names Scryer stores in `title_external_ids`.
fn external_ids(entry: &Value) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    let Some(object) = field(entry, "ProviderIds").and_then(Value::as_object) else {
        return ids;
    };
    for (raw_key, raw_value) in object {
        let key = raw_key.trim().to_ascii_lowercase();
        let Some((_, source)) = PROVIDER_ID_KEYS
            .iter()
            .find(|(candidate, _)| key == *candidate)
        else {
            continue;
        };
        let value = match raw_value {
            Value::String(text) => text.trim().to_string(),
            Value::Number(number) => number.to_string(),
            _ => continue,
        };
        if !value.is_empty() {
            ids.insert((*source).to_string(), value);
        }
    }
    ids
}

/// TMDB/TVDB/IMDb ids from a Plex item's `Guid` list (`tmdb://…`), with the
/// legacy agent `guid` attribute as a second source.
fn plex_external_ids(entry: &Value) -> BTreeMap<String, String> {
    entry
        .get("Guid")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|guid| guid.get("id").and_then(Value::as_str))
        .chain(entry.get("guid").and_then(Value::as_str))
        .filter_map(crate::external_identity::plex_external_id)
        .fold(BTreeMap::new(), |mut ids, id| {
            // The `Guid` list comes first and wins over the legacy attribute.
            ids.entry(id.source).or_insert(id.value);
            ids
        })
}

fn distinct_series_ids(items: &[ProviderPlayedItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.kind == MediaServerSignalKind::Episode)
        .filter_map(|item| item.series_provider_item_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn apply_series_ids(
    items: &mut [ProviderPlayedItem],
    resolved: &BTreeMap<String, BTreeMap<String, String>>,
) {
    for item in items.iter_mut() {
        let Some(series_id) = item.series_provider_item_id.as_deref() else {
            continue;
        };
        if let Some(ids) = resolved.get(series_id) {
            item.series_external_ids = ids.clone();
        }
    }
}

/// A key read exactly, then case-insensitively. Jellyfin and Emby both
/// serialize PascalCase today, but a server or proxy that camel-cases its
/// output must not turn every played item into an unreadable one.
fn field<'a>(entry: &'a Value, name: &str) -> Option<&'a Value> {
    let object = entry.as_object()?;
    object.get(name).or_else(|| {
        object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

/// A JSON field that may be a number or a numeric string, as these servers'
/// serializers have produced both.
fn integer_field(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn string_field(entry: &Value, name: &str) -> Option<String> {
    let value = field(entry, name)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// A non-empty string or a number, as text. Plex sends rating keys as strings
/// and account ids as numbers, and has done each the other way round.
fn scalar_string(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Jellyfin and Emby timestamps are ISO-8601, sometimes without a zone marker.
/// A value that cannot be read becomes `None` rather than "now": a wrong
/// last-played time would make stale data look fresh.
fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc())
}

/// Plex timestamps are Unix seconds. Zero or negative is "never", not 1970.
fn epoch_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let seconds = integer_field(value).filter(|seconds| *seconds > 0)?;
    DateTime::from_timestamp(seconds, 0)
}

// ── URLs ────────────────────────────────────────────────────────────────────

/// A stored base URL with a trailing slash, so `join` keeps its last path
/// segment. Mirrors the handling in [`crate::media_server_playback`].
fn server_base(base_url: &str) -> AppResult<Url> {
    let mut base = Url::parse(base_url.trim()).map_err(|_| {
        AppError::validation_refused(
            crate::error_reasons::IDENTITY_MEDIA_SERVER_BASE_URL_INVALID,
            "media server base URL is invalid",
        )
    })?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base)
}

fn invalid_base_url() -> AppError {
    AppError::validation_refused(
        crate::error_reasons::IDENTITY_MEDIA_SERVER_BASE_URL_INVALID,
        "media server base URL is invalid",
    )
}

fn played_items_url(
    base: &Url,
    external_user_id: &str,
    start_index: usize,
    fields: &str,
) -> AppResult<Url> {
    let mut url = base
        .join(&format!("Users/{external_user_id}/Items"))
        .map_err(|_| invalid_base_url())?;
    url.query_pairs_mut()
        .append_pair("Recursive", "true")
        .append_pair("IsMissing", "false")
        .append_pair("Filters", "IsPlayed")
        .append_pair("IncludeItemTypes", "Movie,Episode")
        .append_pair("Fields", fields)
        .append_pair("EnableUserData", "true")
        .append_pair("StartIndex", &start_index.to_string())
        .append_pair("Limit", &SIGNAL_PAGE_SIZE.to_string());
    Ok(url)
}

/// Look up series rows by id.
///
/// `Recursive=true` is not optional even though the query names exact ids: a
/// non-recursive `Users/{id}/Items` query is scoped to the *user root's direct
/// children*, which on a real server are the top-level library folders, not the
/// series inside them. Without it a server can answer with an empty `Items`
/// array and every episode stays unmapped — the exact silent failure the played
/// items query already sends `Recursive=true` to avoid.
fn series_lookup_url(base: &Url, external_user_id: &str, ids: &[String]) -> AppResult<Url> {
    let mut url = base
        .join(&format!("Users/{external_user_id}/Items"))
        .map_err(|_| invalid_base_url())?;
    url.query_pairs_mut()
        .append_pair("Recursive", "true")
        .append_pair("Ids", &ids.join(","))
        .append_pair("Fields", "ProviderIds")
        .append_pair("Limit", &ids.len().to_string());
    Ok(url)
}

fn plex_url(base: &Url, path: &str) -> AppResult<Url> {
    base.join(path).map_err(|_| invalid_base_url())
}

/// `/library/metadata/{k1,k2,…}` with the `Guid` list switched on.
fn plex_metadata_url(base: &Url, keys: &[String]) -> AppResult<Url> {
    let mut url = plex_url(base, &format!("library/metadata/{}", keys.join(",")))?;
    url.query_pairs_mut().append_pair("includeGuids", "1");
    Ok(url)
}

/// The `MediaContainer` every Plex answer is wrapped in. Without one the body
/// is not a Plex answer at all, and reading it as "nothing" would be a guess.
fn plex_container(body: &Value) -> AppResult<&Value> {
    body.get("MediaContainer")
        .filter(|container| container.is_object())
        .ok_or_else(|| AppError::Repository("unreadable played history".into()))
}

fn plex_metadata_entries(body: &Value) -> AppResult<Vec<Value>> {
    Ok(plex_container(body)?
        .get("Metadata")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

// ── Failure classification ──────────────────────────────────────────────────

/// Why one request failed, classified once where the response is read.
///
/// The reasons it becomes are credential-free on purpose: the request URL
/// carries the participant's id and the headers carry the admin key, and
/// neither belongs in an error that reaches a state row.
#[derive(Debug)]
enum FetchFailure {
    /// The server answered with a non-success status.
    Status(StatusCode),
    /// The request never got an answer.
    Transport(&'static str),
    /// The answer was not JSON.
    Unreadable,
}

impl FetchFailure {
    fn into_app_error(self) -> AppError {
        AppError::Repository(match self {
            Self::Status(status) => format!("status {}", status.as_u16()),
            Self::Transport(reason) => reason.to_string(),
            Self::Unreadable => "unreadable response".to_string(),
        })
    }
}

/// A short, credential-free classification of a transport failure. Never the
/// raw error: it can carry the request URL.
fn transport_reason(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_decode() {
        "unreadable response"
    } else {
        "request failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A full Jellyfin movie row, as the server returns it.
    fn movie_item() -> Value {
        json!({
            "Id": "jf-movie-1",
            "Name": "Example Feature",
            "Type": "Movie",
            "ProviderIds": { "Tmdb": "603", "Imdb": "tt0133093" },
            "UserData": { "Played": true, "PlayCount": 3, "LastPlayedDate": "2026-08-30T21:14:05.0000000Z" }
        })
    }

    fn episode_item() -> Value {
        json!({
            "Id": "jf-episode-1",
            "Name": "Pilot",
            "Type": "Episode",
            "SeriesId": "jf-series-1",
            "SeriesName": "Example Show",
            "ParentIndexNumber": 2,
            "IndexNumber": 5,
            "ProviderIds": { "Tvdb": "999" },
            "UserData": { "Played": true, "PlayCount": 1, "LastPlayedDate": "2026-08-29T10:00:00Z" }
        })
    }

    #[test]
    fn normalizes_a_played_movie() {
        let item = normalize_jellyfin_item(&movie_item()).expect("movie normalizes");
        assert_eq!(item.provider_item_id, "jf-movie-1");
        assert_eq!(item.kind, MediaServerSignalKind::Movie);
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        assert_eq!(
            item.external_ids.get("imdb").map(String::as_str),
            Some("tt0133093")
        );
        assert!(item.played);
        assert_eq!(item.play_count, 3);
        assert!(item.last_played_at.is_some());
        assert!(item.series_provider_item_id.is_none());
    }

    #[test]
    fn normalizes_an_episode_with_its_coordinates() {
        let item = normalize_jellyfin_item(&episode_item()).expect("episode normalizes");
        assert_eq!(item.kind, MediaServerSignalKind::Episode);
        assert_eq!(item.season_number, Some(2));
        assert_eq!(item.episode_number, Some(5));
        assert_eq!(item.series_provider_item_id.as_deref(), Some("jf-series-1"));
        // Series ids are attached by the batched lookup, not by the item row.
        assert!(item.series_external_ids.is_empty());
    }

    #[test]
    fn skips_malformed_and_unsupported_items() {
        let page = vec![
            movie_item(),
            // No Id.
            json!({ "Name": "Nameless", "Type": "Movie" }),
            // A type this wave does not record.
            json!({ "Id": "jf-series-1", "Name": "Example Show", "Type": "Series" }),
            episode_item(),
        ];
        let (items, skipped) = normalize_jellyfin_items(&page);
        assert_eq!(items.len(), 2);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn reads_provider_ids_case_insensitively_and_from_numbers() {
        let item = normalize_jellyfin_item(&json!({
            "Id": "jf-movie-2",
            "Type": "Movie",
            "ProviderIds": { "TMDB": 12345, "Unknown": "x" }
        }))
        .expect("normalizes");
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("12345")
        );
        assert_eq!(item.external_ids.len(), 1);
    }

    #[test]
    fn missing_user_data_does_not_invent_a_last_played_time() {
        let item = normalize_jellyfin_item(&json!({
            "Id": "jf-movie-3",
            "Type": "Movie",
            "ProviderIds": { "Tmdb": "1" }
        }))
        .expect("normalizes");
        assert!(item.last_played_at.is_none());
        assert_eq!(item.play_count, 0);
        // The query filtered on IsPlayed, so an item with no UserData block is
        // still a played item.
        assert!(item.played);
    }

    #[test]
    fn parses_jellyfins_zoneless_timestamps() {
        assert!(parse_timestamp("2026-08-30T21:14:05.0000000").is_some());
        assert!(parse_timestamp("2026-08-30T21:14:05Z").is_some());
        assert!(parse_timestamp("not a date").is_none());
        assert!(parse_timestamp("   ").is_none());
    }

    #[test]
    fn played_items_url_carries_every_required_parameter() {
        let base = server_base("http://jellyfin.example/media").expect("base");
        let url = played_items_url(&base, "user-123", 500, JELLYFIN_ITEM_FIELDS).expect("url");
        assert_eq!(url.path(), "/media/Users/user-123/Items");
        let query = url.query().expect("query");
        for expected in [
            "Recursive=true",
            "Filters=IsPlayed",
            "IncludeItemTypes=Movie%2CEpisode",
            "StartIndex=500",
            "Limit=500",
        ] {
            assert!(query.contains(expected), "missing {expected} in {query}");
        }
        // `Fields` may only name real `ItemFields` members, so it is asserted
        // exactly rather than by substring: an item property that arrives
        // unconditionally must not creep back into the list.
        assert!(
            query.contains("Fields=ProviderIds%2CParentId%2CPath"),
            "unexpected Fields list in {query}"
        );
        // The user-data block is requested by its own switch, not by a field.
        assert!(query.contains("EnableUserData=true"), "{query}");
    }

    /// The series lookup names exact ids, but still has to be recursive: a
    /// non-recursive user-items query only sees the user root's direct children,
    /// which is how every episode silently stays unmapped.
    #[test]
    fn the_series_lookup_url_is_recursive_and_batched() {
        let base = server_base("http://jellyfin.example/media").expect("base");
        let url = series_lookup_url(
            &base,
            "user-123",
            &["jf-series-1".to_string(), "jf-series-2".to_string()],
        )
        .expect("url");
        assert_eq!(url.path(), "/media/Users/user-123/Items");
        let query = url.query().expect("query");
        for expected in [
            "Recursive=true",
            "Ids=jf-series-1%2Cjf-series-2",
            "Fields=ProviderIds",
            "Limit=2",
        ] {
            assert!(query.contains(expected), "missing {expected} in {query}");
        }
    }

    #[tokio::test]
    async fn a_connection_without_a_credential_is_an_error() {
        let source = HttpMediaServerSignalSource::new();
        let mut connection = connection_fixture();
        connection.api_key = None;
        let error = source
            .fetch_played_items(&connection, "user-123")
            .await
            .expect_err("no credential");
        assert!(format!("{error}").contains("no credential"));
    }

    /// Two pages plus the batched series lookup, end to end over HTTP.
    ///
    /// The page size is 500, so the first page is padded to exactly that many
    /// items to prove the adapter keeps paging until a short page arrives, and
    /// that it advances `StartIndex` rather than re-reading page one forever.
    #[tokio::test]
    async fn pages_until_a_short_page_and_resolves_series_ids_in_one_batch() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let mut first_page = (0..SIGNAL_PAGE_SIZE - 1)
            .map(|index| {
                json!({
                    "Id": format!("jf-filler-{index}"),
                    "Type": "Movie",
                    "ProviderIds": { "Tmdb": index.to_string() },
                    "UserData": { "Played": true, "PlayCount": 1 }
                })
            })
            .collect::<Vec<_>>();
        first_page.push(movie_item());

        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("StartIndex", "0"))
            .and(query_param("Filters", "IsPlayed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": first_page,
                "TotalRecordCount": SIGNAL_PAGE_SIZE + 1
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("StartIndex", SIGNAL_PAGE_SIZE.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [episode_item()],
                "TotalRecordCount": SIGNAL_PAGE_SIZE + 1
            })))
            .mount(&server)
            .await;

        // The series lookup is the only request carrying `Ids`, and it must be
        // recursive or a real server answers out of the user root's direct
        // children — which never contain a series.
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("Ids", "jf-series-1"))
            .and(query_param("Recursive", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{
                    "Id": "jf-series-1",
                    "Type": "Series",
                    "ProviderIds": { "Tvdb": "999" }
                }]
            })))
            .mount(&server)
            .await;

        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .fetch_played_items(&connection, "user-123")
            .await
            .expect("two pages read");

        assert_eq!(items.len(), SIGNAL_PAGE_SIZE + 1);
        let episode = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Episode)
            .expect("the second page's episode");
        // The series ids arrived from the batched lookup, not from the episode.
        assert_eq!(
            episode.series_external_ids.get("tvdb").map(String::as_str),
            Some("999")
        );
        // The episode's own coordinates came off the item row, which is why the
        // `Fields` list does not have to ask for them: the fixture returns
        // `SeriesId`, `IndexNumber`, and `ParentIndexNumber` as a real server
        // does, and the trimmed request still reads them.
        assert_eq!(episode.season_number, Some(2));
        assert_eq!(episode.episode_number, Some(5));
        assert_eq!(
            episode.series_provider_item_id.as_deref(),
            Some("jf-series-1")
        );
        // `EnableUserData=true`, not a `UserData` field name, is what fills this.
        assert_eq!(episode.play_count, 1);
        assert!(episode.played);
    }

    /// A series lookup that resolves nothing is not fatal: the read still
    /// succeeds and the episodes stay honestly unmapped rather than being given
    /// ids nobody returned.
    #[tokio::test]
    async fn a_series_lookup_that_resolves_nothing_leaves_episodes_unmapped() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("StartIndex", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [episode_item()],
                "TotalRecordCount": 1
            })))
            .mount(&server)
            .await;

        // What a non-recursive query against a real user root looks like: a 200
        // that resolved none of the requested ids.
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("Ids", "jf-series-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Items": [] })))
            .mount(&server)
            .await;

        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .fetch_played_items(&connection, "user-123")
            .await
            .expect("an unresolved series batch is not fatal");

        assert_eq!(items.len(), 1);
        assert!(
            items[0].series_external_ids.is_empty(),
            "an unresolved series must not be given invented ids"
        );
    }

    /// A page the server refuses fails the participant rather than reporting a
    /// short history, and the reason never carries the URL or the credential.
    #[tokio::test]
    async fn an_http_failure_is_an_error_with_a_credential_free_reason() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let error = HttpMediaServerSignalSource::new()
            .fetch_played_items(&connection, "user-123")
            .await
            .expect_err("401 fails the read");
        let message = format!("{error}");
        assert!(message.contains("status 401"), "{message}");
        assert!(!message.contains("api-key"), "{message}");
        assert!(!message.contains("user-123"), "{message}");
    }

    #[tokio::test]
    async fn incomplete_played_histories_fail_instead_of_publishing_partial_signals() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for body in [
            json!({ "Items": [movie_item(), { "Type": "Movie" }] }),
            json!({ "Unexpected": [] }),
            json!({ "Items": vec![movie_item(); SIGNAL_PAGE_SIZE] }),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            let mut connection = connection_fixture();
            connection.base_url = server.uri();
            HttpMediaServerSignalSource::new()
                .fetch_played_items(&connection, "user-123")
                .await
                .expect_err("an unreadable or unbounded history is not a fresh complete snapshot");
        }
    }

    // ── Emby ────────────────────────────────────────────────────────────────

    fn emby_movie_item() -> Value {
        json!({
            "Id": "emby-movie-1",
            "Name": "Example Feature",
            "Type": "Movie",
            "ProviderIds": { "Tmdb": "603", "Imdb": "tt0133093" },
            "UserData": {
                "Played": true,
                "PlayCount": 4,
                "LastPlayedDate": "2026-08-30T21:14:05.0000000Z",
                "IsFavorite": true
            }
        })
    }

    fn emby_episode_item() -> Value {
        json!({
            "Id": "emby-episode-1",
            "Name": "Pilot",
            "Type": "Episode",
            "SeriesId": "emby-series-1",
            "ParentIndexNumber": 1,
            "IndexNumber": 3,
            "UserData": { "Played": true, "PlayCount": 1, "IsFavorite": false }
        })
    }

    fn emby_connection(base_url: String) -> MediaServerConnection {
        let mut connection = connection_fixture();
        connection.provider = MediaServerProvider::Emby;
        connection.display_name = "Example Emby".into();
        connection.base_url = base_url;
        connection
    }

    #[test]
    fn reads_the_favourite_flag_and_tolerates_camel_cased_keys() {
        let item = normalize_jellyfin_item(&emby_movie_item()).expect("movie normalizes");
        assert!(item.is_favorite);
        assert_eq!(item.play_count, 4);

        let item = normalize_jellyfin_item(&json!({
            "id": "emby-movie-2",
            "type": "movie",
            "providerIds": { "tmdb": "77" },
            "userData": {
                "played": true,
                "playCount": 2,
                "lastPlayedDate": "2026-08-01T10:00:00Z",
                "isFavorite": true
            }
        }))
        .expect("camel-cased item normalizes");
        assert_eq!(item.provider_item_id, "emby-movie-2");
        assert_eq!(item.kind, MediaServerSignalKind::Movie);
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("77")
        );
        assert_eq!(item.play_count, 2);
        assert!(item.last_played_at.is_some());
        assert!(item.is_favorite);

        // No flag on record is not a favourite.
        let item = normalize_jellyfin_item(&movie_item()).expect("movie normalizes");
        assert!(!item.is_favorite);
    }

    /// Emby is read with its own credential header, from the stored API base
    /// (which may carry the `/emby` prefix), and its series are resolved in
    /// one batch exactly as Jellyfin's are.
    #[tokio::test]
    async fn emby_reads_played_items_with_its_own_credential() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/emby/Users/emby-user-1/Items"))
            .and(header("X-Emby-Token", "api-key"))
            .and(query_param("StartIndex", "0"))
            .and(query_param("Filters", "IsPlayed"))
            .and(query_param("Recursive", "true"))
            .and(query_param("IncludeItemTypes", "Movie,Episode"))
            .and(query_param("Fields", "ProviderIds"))
            .and(query_param("EnableUserData", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [emby_movie_item(), emby_episode_item()],
                "TotalRecordCount": 2
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emby/Users/emby-user-1/Items"))
            .and(header("X-Emby-Token", "api-key"))
            .and(query_param("Ids", "emby-series-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{ "Id": "emby-series-1", "Type": "Series", "ProviderIds": { "Tvdb": "4242" } }]
            })))
            .mount(&server)
            .await;

        let items = HttpMediaServerSignalSource::new()
            .fetch_played_items(
                &emby_connection(format!("{}/emby", server.uri())),
                "emby-user-1",
            )
            .await
            .expect("Emby history reads");

        assert_eq!(items.len(), 2);
        let movie = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Movie)
            .expect("the movie");
        assert!(movie.played);
        assert_eq!(movie.play_count, 4);
        assert_eq!(
            movie.last_played_at,
            Some(
                DateTime::parse_from_rfc3339("2026-08-30T21:14:05Z")
                    .expect("fixture time")
                    .with_timezone(&Utc)
            )
        );
        assert!(movie.is_favorite);
        assert_eq!(
            movie.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        assert_eq!(
            movie.external_ids.get("imdb").map(String::as_str),
            Some("tt0133093")
        );

        let episode = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Episode)
            .expect("the episode");
        assert_eq!(episode.season_number, Some(1));
        assert_eq!(episode.episode_number, Some(3));
        assert!(!episode.is_favorite);
        assert_eq!(
            episode.series_external_ids.get("tvdb").map(String::as_str),
            Some("4242")
        );
    }

    /// Every partial Emby read — an unreadable item, a cap hit, a page the
    /// server refuses after a full one — fails as incomplete rather than
    /// reporting the part it did read.
    #[tokio::test]
    async fn emby_partial_reads_are_incomplete() {
        use wiremock::matchers::{method, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // An unreadable item beside a good one.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [emby_movie_item(), { "Type": "Movie" }]
            })))
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .fetch_played_items(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("an unreadable item makes the read incomplete");
        assert!(
            format!("{error}").contains("incomplete played history: unreadable items"),
            "{error}"
        );

        // A server that always has another full page.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": vec![emby_movie_item(); SIGNAL_PAGE_SIZE]
            })))
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .fetch_played_items(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("the cap makes the read incomplete");
        assert!(
            format!("{error}").contains("incomplete played history: item cap reached"),
            "{error}"
        );

        // A full first page, then a refused second one.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": vec![emby_movie_item(); SIGNAL_PAGE_SIZE]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", SIGNAL_PAGE_SIZE.to_string()))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .fetch_played_items(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("a refused page fails the read");
        assert!(format!("{error}").contains("status 503"), "{error}");
    }

    // ── Plex ────────────────────────────────────────────────────────────────

    const PLEX_OWNER_ID: &str = "1001";
    const PLEX_SHARED_ID: &str = "2002";
    const PLEX_TOKEN: &str = "plex-token";

    fn plex_connection(base_url: String) -> MediaServerConnection {
        let mut connection = connection_fixture();
        connection.provider = MediaServerProvider::Plex;
        connection.display_name = "Example Plex".into();
        connection.base_url = base_url;
        connection.api_key = Some(PLEX_TOKEN.into());
        connection.machine_id = Some("plex-machine-1".into());
        connection
    }

    /// The account service and the media server, both answered by one mock.
    fn plex_source(server: &wiremock::MockServer) -> HttpMediaServerSignalSource {
        HttpMediaServerSignalSource::with_plex_service_url(
            Url::parse(&format!("{}/", server.uri())).expect("mock URL"),
        )
    }

    async fn mount_plex_owner(server: &wiremock::MockServer) {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": 1001, "uuid": "plex-owner-uuid", "username": "example-owner" }
            })))
            .mount(server)
            .await;
    }

    fn plex_movie(view_count: i64, last_viewed_at: i64) -> Value {
        json!({
            "ratingKey": "101",
            "type": "movie",
            "title": "Example Feature",
            "guid": "plex://movie/example",
            "Guid": [
                { "id": "imdb://tt0133093" },
                { "id": "tmdb://603" },
                { "id": "tvdb://1" }
            ],
            "viewCount": view_count,
            "lastViewedAt": last_viewed_at
        })
    }

    fn plex_episode() -> Value {
        json!({
            "ratingKey": "201",
            "type": "episode",
            "title": "Pilot",
            "grandparentRatingKey": "200",
            "grandparentTitle": "Example Show",
            "parentIndex": 2,
            "index": 5,
            "Guid": [{ "id": "tvdb://7001" }],
            "viewCount": 1,
            "lastViewedAt": 1_756_000_000
        })
    }

    async fn mount_plex_show_metadata(server: &wiremock::MockServer) {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/library/metadata/200"))
            .and(query_param("includeGuids", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "Metadata": [{
                    "ratingKey": "200",
                    "type": "show",
                    "Guid": [{ "id": "tvdb://999" }, { "id": "tmdb://1399" }]
                }] }
            })))
            .mount(server)
            .await;
    }

    #[test]
    fn normalizes_a_plex_item_from_its_guid_list() {
        let item = normalize_plex_item(&plex_movie(3, 1_756_588_445)).expect("movie normalizes");
        assert_eq!(item.provider_item_id, "101");
        assert_eq!(item.kind, MediaServerSignalKind::Movie);
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        assert_eq!(
            item.external_ids.get("imdb").map(String::as_str),
            Some("tt0133093")
        );
        assert!(item.played);
        assert_eq!(item.play_count, 3);
        assert_eq!(
            item.last_played_at,
            DateTime::from_timestamp(1_756_588_445, 0)
        );
        assert!(!item.is_favorite, "Plex has no favourite");
        assert!(item.series_provider_item_id.is_none());

        let episode = normalize_plex_item(&plex_episode()).expect("episode normalizes");
        assert_eq!(episode.series_provider_item_id.as_deref(), Some("200"));
        assert_eq!(episode.season_number, Some(2));
        assert_eq!(episode.episode_number, Some(5));

        assert!(normalize_plex_item(&json!({ "type": "movie" })).is_none());
        assert!(normalize_plex_item(&json!({ "ratingKey": "9", "type": "track" })).is_none());
        // A never-viewed timestamp is not 1970.
        assert!(
            normalize_plex_item(&plex_movie(1, 0))
                .expect("normalizes")
                .last_played_at
                .is_none()
        );
    }

    #[test]
    fn folds_plex_history_into_plays_per_item() {
        let (plays, skipped) = fold_plex_history(&[
            json!({ "ratingKey": "101", "type": "movie", "viewedAt": 1_700_000_000 }),
            json!({ "ratingKey": "101", "type": "movie", "viewedAt": 1_700_500_000 }),
            json!({ "ratingKey": 201, "type": "episode", "viewedAt": "1700100000" }),
            // Music is not a movie or episode play: passed over, not unreadable.
            json!({ "ratingKey": "301", "type": "track", "viewedAt": 1_700_000_000 }),
            // No rating key, and no type: both unreadable.
            json!({ "type": "movie", "viewedAt": 1_700_000_000 }),
            json!({ "ratingKey": "102", "viewedAt": 1_700_000_000 }),
        ]);
        assert_eq!(skipped, 2);
        assert_eq!(plays.len(), 2);
        assert_eq!(
            plays.get("101"),
            Some(&PlexPlays {
                count: 2,
                last_viewed_at: DateTime::from_timestamp(1_700_500_000, 0),
            })
        );
        assert_eq!(plays.get("201").map(|play| play.count), Some(1));
    }

    /// The owner is read from the library: one movie and one show section,
    /// paged with the container headers, the watched filter, and the `Guid`
    /// opt-in; a music section is not asked.
    #[tokio::test]
    async fn plex_owner_reads_the_library_watched_set() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_plex_show_metadata(&server).await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 3, "Directory": [
                    { "key": "1", "type": "movie", "title": "Movies" },
                    { "key": 2, "type": "show", "title": "Shows" },
                    { "key": "3", "type": "artist", "title": "Music" }
                ] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .and(header("X-Plex-Container-Start", "0"))
            .and(header(
                "X-Plex-Container-Size",
                SIGNAL_PAGE_SIZE.to_string(),
            ))
            .and(query_param("type", "1"))
            .and(query_param("viewCount>>", "0"))
            .and(query_param("includeGuids", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "offset": 0, "totalSize": 1,
                    "Metadata": [plex_movie(3, 1_756_588_445)] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/2/all"))
            .and(query_param("type", "4"))
            .and(query_param("viewCount>>", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "offset": 0, "totalSize": 1,
                    "Metadata": [plex_episode()] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let items = plex_source(&server)
            .fetch_played_items(&plex_connection(server.uri()), PLEX_OWNER_ID)
            .await
            .expect("the owner's watched set reads");

        assert_eq!(items.len(), 2);
        let movie = &items[0];
        assert_eq!(movie.provider_item_id, "101");
        assert_eq!(movie.play_count, 3);
        assert_eq!(
            movie.last_played_at,
            DateTime::from_timestamp(1_756_588_445, 0)
        );
        assert_eq!(
            movie.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        assert!(!movie.is_favorite);
        let episode = &items[1];
        assert_eq!(episode.kind, MediaServerSignalKind::Episode);
        assert_eq!(
            episode.series_external_ids.get("tvdb").map(String::as_str),
            Some("999")
        );
        assert_eq!(
            episode.series_external_ids.get("tmdb").map(String::as_str),
            Some("1399")
        );
    }

    /// A shared user is read from server history by their own account id.
    /// Their play count and last play come from their history, never from the
    /// owner-token metadata; an item that has left the library is dropped; a
    /// batch refused as not found is retried key by key.
    #[tokio::test]
    async fn plex_shared_user_reads_history_by_account_id() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_plex_show_metadata(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .and(header("X-Plex-Container-Start", "0"))
            .and(query_param("accountID", PLEX_SHARED_ID))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 5, "offset": 0, "totalSize": 5, "Metadata": [
                    { "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_500_000 },
                    { "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 },
                    { "ratingKey": "201", "type": "episode", "accountID": 2002, "viewedAt": 1_700_100_000,
                      "grandparentKey": "/library/metadata/200" },
                    { "ratingKey": "301", "type": "track", "accountID": 2002, "viewedAt": 1_700_000_000 },
                    { "ratingKey": "404", "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 }
                ] }
            })))
            .mount(&server)
            .await;
        // The batch names an item that has left the library, so the server
        // refuses the whole batch.
        Mock::given(method("GET"))
            .and(path("/library/metadata/101,201,404"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101"))
            .and(query_param("includeGuids", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                // The owner's own view count: it must not leak into the
                // shared user's facts.
                "MediaContainer": { "size": 1, "Metadata": [plex_movie(9, 1_760_000_000)] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/201"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "Metadata": [plex_episode()] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/404"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        // The owner's watched set is never read for a shared user.
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let items = plex_source(&server)
            .fetch_played_items(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect("the shared user's history reads");

        assert_eq!(items.len(), 2);
        let movie = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Movie)
            .expect("the movie");
        assert!(movie.played);
        assert_eq!(movie.play_count, 2);
        assert_eq!(
            movie.last_played_at,
            DateTime::from_timestamp(1_700_500_000, 0)
        );
        assert_eq!(
            movie.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        let episode = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Episode)
            .expect("the episode");
        assert_eq!(episode.play_count, 1);
        assert_eq!(episode.season_number, Some(2));
        assert_eq!(episode.episode_number, Some(5));
        assert_eq!(
            episode.series_external_ids.get("tvdb").map(String::as_str),
            Some("999")
        );
    }

    /// Every partial Plex read fails as incomplete rather than reporting what
    /// it managed to read.
    #[tokio::test]
    async fn plex_partial_reads_are_incomplete() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        async fn history_error(body: ResponseTemplate) -> String {
            let server = MockServer::start().await;
            mount_plex_owner(&server).await;
            Mock::given(method("GET"))
                .and(path("/status/sessions/history/all"))
                .respond_with(body)
                .mount(&server)
                .await;
            let error = plex_source(&server)
                .fetch_played_items(&plex_connection(server.uri()), PLEX_SHARED_ID)
                .await
                .expect_err("a partial history is not a complete one");
            format!("{error}")
        }

        // An entry with no rating key.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": 1, "Metadata": [
                { "type": "movie", "viewedAt": 1_700_000_000 }
            ] }
        })))
        .await;
        assert!(
            error.contains("incomplete played history: unreadable items"),
            "{error}"
        );

        // A server that always has another full page.
        let full_page = (0..SIGNAL_PAGE_SIZE)
            .map(|index| json!({ "ratingKey": index.to_string(), "type": "movie", "viewedAt": 1_700_000_000 }))
            .collect::<Vec<_>>();
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": SIGNAL_PAGE_SIZE, "Metadata": full_page }
        })))
        .await;
        assert!(
            error.contains("incomplete played history: item cap reached"),
            "{error}"
        );

        // A server that ignores the paging headers and answers from the top.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": SIGNAL_PAGE_SIZE, "offset": 0, "Metadata": full_page }
        })))
        .await;
        assert!(error.contains("paging was not honoured"), "{error}");

        // A body that is not a Plex answer.
        let error =
            history_error(ResponseTemplate::new(200).set_body_json(json!({ "Unexpected": [] })))
                .await;
        assert!(error.contains("unreadable played history"), "{error}");

        // A refused request.
        let error = history_error(ResponseTemplate::new(401)).await;
        assert!(error.contains("status 401"), "{error}");
        assert!(!error.contains(PLEX_TOKEN), "{error}");

        // An unreadable item in the owner's watched set.
        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "Directory": [{ "key": "1", "type": "movie" }] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 2, "Metadata": [
                    plex_movie(1, 1_700_000_000),
                    { "type": "movie", "viewCount": 1 }
                ] }
            })))
            .mount(&server)
            .await;
        let error = plex_source(&server)
            .fetch_played_items(&plex_connection(server.uri()), PLEX_OWNER_ID)
            .await
            .expect_err("an unreadable library item makes the read incomplete");
        assert!(
            format!("{error}").contains("incomplete played history: unreadable items"),
            "{error}"
        );
    }

    /// A shared user whose linked id is not a server account id would match
    /// nobody in history; that is an error, never an empty history. So is a
    /// connection with no server selected.
    #[tokio::test]
    async fn plex_reads_that_could_only_come_back_empty_are_errors() {
        use wiremock::MockServer;

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        plex_source(&server)
            .fetch_played_items(&plex_connection(server.uri()), "plex-other-uuid")
            .await
            .expect_err("a non-numeric account id cannot filter history");

        let error = plex_source(&server)
            .fetch_played_items(&plex_connection("https://plex.tv".into()), PLEX_SHARED_ID)
            .await
            .expect_err("plex.tv is not a media server");
        assert!(format!("{error}").contains("no Plex server selected"));
    }

    fn connection_fixture() -> MediaServerConnection {
        MediaServerConnection {
            id: "conn-1".into(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Example Jellyfin".into(),
            base_url: "http://jellyfin.example".into(),
            external_url: None,
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: Default::default(),
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("api-key".into()),
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}
