//! Provider adapters for media-server watch signals (RFC 137 section 7.3).
//!
//! One [`MediaServerSignalSource`] implementation that dispatches on the
//! connection's provider: Jellyfin, Emby, and Plex. [`MediaServerSignalSource::open`]
//! does the per-connection work once (the credential, the server address, and
//! for Plex the server and owner lookups) and hands back a reader that answers
//! each participant with their complete played set or a coded
//! [`SignalReadFailure`]. A partial read is never an answer, because "this read
//! stopped early" and "this person has watched nothing" must never be the same
//! answer.
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
//! # Paging
//!
//! Every listing is paged until the server's own total is reached
//! (`TotalRecordCount` on Jellyfin and Emby, `totalSize` on Plex). A server
//! that returns fewer rows than asked for, because it clamps the page size, is
//! simply asked for the next page; an empty page before the total is a paging
//! failure, not the end. Only a server that reports no total is taken at its
//! word that a short page is the last one.
//!
//! A server can also ignore the start it was asked for and still stay inside
//! its total. That is caught three ways: an item seen on an earlier page
//! (Jellyfin and Emby `Id`, Plex watched-set `ratingKey`), a `StartIndex` echo
//! that differs from the one asked for (Jellyfin echoes it; Emby does not
//! document one, so it is checked only when present), and a Plex page that is
//! one of several but carries no `offset`. Plex history repeats items by
//! design, so it relies on the offset alone. Each fails as a paging failure.
//!
//! Two bounds, both deliberate:
//!
//! * a **page size** so a large library does not arrive as one enormous body;
//! * a **hard cap** on items per participant. The read fails as incomplete only
//!   when the cap is reached and another page remains, so the previous watch
//!   history and its freshness timestamp are preserved rather than replaced by
//!   a truncated one.
//!
//! # File paths
//!
//! Each item carries the server's file paths (Jellyfin and Emby `Path`, Plex
//! `Media[].Part[].file`) only so the sweep can tell a film outside every
//! Scryer library from an unmatched one inside it. They are never stored or
//! logged.
//!
//! # Plex
//!
//! A Plex connection's base URL is either the server itself, or plex.tv with the
//! server chosen by its machine id; the latter is resolved to the server's own
//! address through the account's resource list, once per connection per sweep.
//!
//! Plex keeps watch state per account, and the stored credential is the server
//! owner's token. Server history names the owner as account `1` and every
//! shared user by their plex.tv account id, so the owner's plex.tv id is read
//! once per connection (`users/account.json`, the same call that verified the
//! link) to tell the two apart:
//!
//! * **the owner** is read from the library itself: each movie and show
//!   section's `/library/sections/{key}/all` with `type=1` (movies) or `type=4`
//!   (episodes), `viewCount>>=0` (Plex's "greater than"), and `includeGuids=1`.
//!   `viewCount` and `lastViewedAt` are the owner's own. A row with no views
//!   means the filter was not applied, and the read fails rather than trusting
//!   the listing as a played set.
//! * **a user the server is shared with** is read the same way, from the
//!   library, with that user's own access token for this server. plex.tv lists
//!   those tokens at `api/servers/{machineId}/shared_servers` for the owner's
//!   token (`userID` and `accessToken` on each entry). The tokens are used for
//!   this sweep only and are never stored.
//! * **anyone else** (a managed home user, who has no shared-server entry, or
//!   every non-owner when plex.tv cannot list the tokens) is read from
//!   `/status/sessions/history/all` filtered by `accountID`. Plays are folded
//!   per `ratingKey` into a count and the newest `viewedAt`, then the items are
//!   looked up in batches through `/library/metadata/{keys}` for their `Guid`
//!   list and episode coordinates. A history entry whose item has left the
//!   library has nothing to map and is dropped.
//!
//! Listings page with the `X-Plex-Container-Start` and `X-Plex-Container-Size`
//! request headers. Plex has no favourite, so Plex items always report
//! `is_favorite = false`. No user password is ever stored or sent.
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
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use reqwest::StatusCode;
use scryer_application::media_server_signals::reasons;
use scryer_application::{
    MediaServerSignalReader, MediaServerSignalSource, ProviderPlayedItem, SignalReadFailure,
};
use scryer_domain::{MediaServerConnection, MediaServerProvider, MediaServerSignalKind};
use scryer_outbound_http::generic_reqwest_client;
use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::warn;
use url::Url;

use crate::external_identity::{PlexServerDiscoveryFailure, discover_plex_server_base_url};
use crate::jellyfin::JellyfinAuth;

type ReadResult<T> = Result<T, SignalReadFailure>;

/// Per-request budget. Higher than the playback probe's five seconds: this is a
/// paged library query, not a memory read, but it still sits inside a
/// background job that must not hang for minutes.
const SIGNAL_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Items requested per page, for every provider.
pub const SIGNAL_PAGE_SIZE: usize = 500;

/// Hard ceiling on distinct items read for one participant in one sweep.
/// Reaching it with another page still to read fails the read as incomplete
/// rather than publishing a truncated history.
pub const SIGNAL_MAX_ITEMS: usize = 20_000;

/// Plex history has one row per play, not per item, so its rows have their own
/// ceiling: generous enough for a heavy rewatcher, still bounded.
pub const SIGNAL_MAX_HISTORY_ROWS: usize = 200_000;

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

/// Emby's list: the provider ids, and `Path` so an unmatched item's file can be
/// checked against Scryer's libraries. Both are documented `Fields` options on
/// Emby's `/Users/{UserId}/Items`. The user-data block is switched on by
/// `EnableUserData=true`, as on Jellyfin.
const EMBY_ITEM_FIELDS: &str = "ProviderIds,Path";

/// External id sources Scryer joins on, mapped from Jellyfin's and Emby's
/// `ProviderIds` keys. The capitalization has drifted across versions, so the
/// match is case-insensitive on the key rather than a fixed spelling.
const PROVIDER_ID_KEYS: [(&str, &str); 3] = [("tmdb", "tmdb"), ("tvdb", "tvdb"), ("imdb", "imdb")];

/// Plex's account service, where the owner's account id, the server's address,
/// and shared users' server tokens are read.
const PLEX_SERVICE_URL: &str = "https://plex.tv/";

/// A Plex connection whose base URL is the account service names its server by
/// machine id rather than by address.
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

/// Whether a listing has another page, from what the last page said.
///
/// With a reported total, the listing continues until it is reached, whatever
/// size the pages came back at; an empty page short of it is a paging failure.
/// Without one, only a full page implies another.
fn another_page_remains(read: usize, page_len: usize, total: Option<i64>) -> ReadResult<bool> {
    match total {
        Some(total) if (read as i64) < total => {
            if page_len == 0 {
                Err(SignalReadFailure::new(
                    reasons::PAGING_NOT_HONOURED,
                    "incomplete played history: the server stopped before its reported total",
                ))
            } else {
                Ok(true)
            }
        }
        Some(_) => Ok(false),
        None => Ok(page_len >= SIGNAL_PAGE_SIZE),
    }
}

/// `cap` is the bound that was actually hit: items, or Plex history rows.
fn item_cap_reached(connection: &MediaServerConnection, cap: usize) -> SignalReadFailure {
    warn!(
        connection_id = connection.id.as_str(),
        provider = connection.provider.as_str(),
        cap,
        "played history exceeded the per-participant cap; the read was stopped"
    );
    SignalReadFailure::new(
        reasons::ITEM_CAP_REACHED,
        "incomplete played history: item cap reached",
    )
}

fn unreadable_items() -> SignalReadFailure {
    SignalReadFailure::new(
        reasons::ITEMS_UNREADABLE,
        "incomplete played history: unreadable items",
    )
}

fn paging_not_honoured() -> SignalReadFailure {
    SignalReadFailure::new(
        reasons::PAGING_NOT_HONOURED,
        "incomplete played history: paging was not honoured",
    )
}

fn unreadable_listing() -> SignalReadFailure {
    SignalReadFailure::new(reasons::RESPONSE_UNREADABLE, "unreadable played history")
}

#[derive(Clone)]
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
    ) -> ReadResult<Value> {
        self.request_json(url, auth, page)
            .await
            .map_err(FetchFailure::into_failure)
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
        base: &Url,
        auth: SignalAuth<'_>,
        fields: &str,
        external_user_id: &str,
    ) -> ReadResult<Vec<ProviderPlayedItem>> {
        let mut items: Vec<ProviderPlayedItem> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut skipped = 0_usize;
        let mut start_index = 0_usize;

        loop {
            if start_index >= SIGNAL_MAX_ITEMS {
                return Err(item_cap_reached(connection, SIGNAL_MAX_ITEMS));
            }
            let url = played_items_url(base, external_user_id, start_index, fields)?;
            let body = self.fetch_json(url, auth, None).await?;
            // Jellyfin echoes the page's start; a server that answers from
            // elsewhere is not paging as asked. Emby does not echo it.
            if field(&body, "StartIndex")
                .and_then(integer_field)
                .is_some_and(|echoed| echoed != start_index as i64)
            {
                return Err(paging_not_honoured());
            }
            let page = field(&body, "Items")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(unreadable_listing)?;
            let total = field(&body, "TotalRecordCount").and_then(integer_field);

            let (mut normalized, page_skipped) = normalize_jellyfin_items(&page);
            skipped += page_skipped;
            // A server that ignores `StartIndex` but stays inside its total
            // would otherwise hand back the same rows until the total is
            // reached, and the items it never returned would read as unwatched.
            if normalized
                .iter()
                .any(|item| !seen.insert(item.provider_item_id.clone()))
            {
                return Err(paging_not_honoured());
            }
            items.append(&mut normalized);

            start_index += page.len();
            if !another_page_remains(start_index, page.len(), total)? {
                break;
            }
        }

        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                provider = connection.provider.as_str(),
                skipped,
                "skipped unreadable played items"
            );
            return Err(unreadable_items());
        }

        self.attach_series_ids(base, auth, external_user_id, &mut items)
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

    /// The address of the connection's Plex server. A connection made through
    /// plex.tv names its server by machine id; that is resolved to the
    /// server's own address through the owner's resource list.
    async fn plex_server(
        &self,
        connection: &MediaServerConnection,
        base: Url,
        token: &str,
    ) -> ReadResult<Url> {
        let through_service = base
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(PLEX_SERVICE_HOST));
        if !through_service {
            return Ok(base);
        }
        let machine_id = connection
            .machine_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SignalReadFailure::new(reasons::PLEX_SERVER_NOT_SELECTED, "no Plex server selected")
            })?;
        discover_plex_server_base_url(
            &self.client,
            &self.plex_service_url,
            machine_id,
            token,
            Some(SIGNAL_FETCH_TIMEOUT),
        )
        .await
        .map_err(|failure| match failure {
            PlexServerDiscoveryFailure::Transport(error) => SignalReadFailure::new(
                reasons::REQUEST_FAILED,
                format!("Plex server lookup: {}", transport_reason(&error)),
            ),
            PlexServerDiscoveryFailure::Status(status) => {
                FetchFailure::Status(status).into_failure()
            }
            PlexServerDiscoveryFailure::Unreadable(_) => {
                SignalReadFailure::new(reasons::RESPONSE_UNREADABLE, "unreadable Plex server list")
            }
            PlexServerDiscoveryFailure::NoServerUri => SignalReadFailure::new(
                reasons::PLEX_SERVER_UNREACHABLE,
                "the selected Plex server has no reachable address",
            ),
        })
    }

    /// The plex.tv ids of the account the stored token belongs to: `id`, and
    /// `uuid` as the fallback the link verification also accepts.
    async fn plex_owner_account_ids(&self, token: &str) -> ReadResult<Vec<String>> {
        let url = self
            .plex_service_url
            .join("users/account.json")
            .map_err(|_| unreadable_listing())?;
        let body = self.fetch_json(url, SignalAuth::Plex(token), None).await?;
        let ids = body
            .get("user")
            .map(|user| {
                ["id", "uuid"]
                    .into_iter()
                    .filter_map(|key| user.get(key).and_then(scalar_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Err(SignalReadFailure::new(
                reasons::RESPONSE_UNREADABLE,
                "unreadable Plex account",
            ));
        }
        Ok(ids)
    }

    /// The server access tokens of the users `machine_id` is shared with, keyed
    /// by their plex.tv account id, as plex.tv lists them to the owner.
    async fn plex_shared_server_tokens(
        &self,
        machine_id: &str,
        owner_token: &str,
    ) -> Result<BTreeMap<String, String>, FetchFailure> {
        let mut url = self.plex_service_url.clone();
        url.path_segments_mut()
            .map_err(|_| FetchFailure::Unreadable)?
            .pop_if_empty()
            .extend(["api", "servers", machine_id, "shared_servers"]);
        let response = self
            .client
            .get(url)
            .header("Accept", "application/xml")
            .header("X-Plex-Token", owner_token)
            .timeout(SIGNAL_FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|error| FetchFailure::Transport(transport_reason(&error)))?;
        if !response.status().is_success() {
            return Err(FetchFailure::Status(response.status()));
        }
        let body = response
            .text()
            .await
            .map_err(|_| FetchFailure::Unreadable)?;
        shared_server_tokens(&body).ok_or(FetchFailure::Unreadable)
    }

    /// One account's played set, straight from the movie and show libraries,
    /// read with that account's own token.
    async fn fetch_plex_watched_set(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        auth: SignalAuth<'_>,
    ) -> ReadResult<Vec<ProviderPlayedItem>> {
        let sections = self
            .fetch_json(plex_url(base, "library/sections")?, auth, None)
            .await?;
        let directories = plex_container(&sections)?
            .get("Directory")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut items = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut repeated = false;
        let mut skipped = 0_usize;
        let mut unplayed = 0_usize;
        for directory in directories {
            let plex_type = match directory.get("type").and_then(Value::as_str) {
                Some("movie") => PLEX_TYPE_MOVIE,
                Some("show") => PLEX_TYPE_EPISODE,
                _ => continue,
            };
            let Some(key) = directory.get("key").and_then(scalar_string) else {
                // A movie or show library Scryer cannot address is a library
                // whose plays would silently go missing.
                return Err(SignalReadFailure::new(
                    reasons::RESPONSE_UNREADABLE,
                    "incomplete played history: unreadable library",
                ));
            };
            let mut url = plex_url(base, &format!("library/sections/{key}/all"))?;
            url.query_pairs_mut()
                .append_pair("type", plex_type)
                .append_pair("viewCount>>", "0")
                .append_pair("includeGuids", "1");
            // A section that starts at the cap is still read: the cap fails
            // the read only when a non-empty page leaves another to read.
            self.for_each_plex_page(connection, url, auth, |entries| {
                for entry in &entries {
                    match normalize_plex_item(entry) {
                        Some(item) if !seen.insert(item.provider_item_id.clone()) => {
                            repeated = true;
                        }
                        Some(item) if !item.played => unplayed += 1,
                        Some(item) => items.push(item),
                        None => skipped += 1,
                    }
                }
                if repeated {
                    // A rating key is one item in one section; seeing it again
                    // means a page was answered from elsewhere.
                    return Err(paging_not_honoured());
                }
                Ok((items.len() + skipped + unplayed >= SIGNAL_MAX_ITEMS)
                    .then_some(SIGNAL_MAX_ITEMS))
            })
            .await?;
        }

        if unplayed > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                unplayed, "the Plex watched-set listing returned unwatched items"
            );
            return Err(SignalReadFailure::new(
                reasons::FILTER_NOT_HONOURED,
                "incomplete played history: the watched filter was not applied",
            ));
        }
        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped, "skipped unreadable Plex played items"
            );
            return Err(unreadable_items());
        }
        Ok(items)
    }

    /// An account's plays, from server history filtered to that account.
    async fn fetch_plex_history(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        auth: SignalAuth<'_>,
        account_id: &str,
    ) -> ReadResult<Vec<ProviderPlayedItem>> {
        let mut url = plex_url(base, "status/sessions/history/all")?;
        url.query_pairs_mut()
            .append_pair("accountID", account_id)
            .append_pair("sort", "viewedAt:desc");

        // Folded page by page: the cap is on distinct items, and the rows
        // themselves (one per play) are not kept.
        let mut plays: BTreeMap<String, PlexPlays> = BTreeMap::new();
        let mut skipped = 0_usize;
        let mut rows = 0_usize;
        // Rows repeat rating keys by design, one per play, so a page answered
        // from elsewhere is caught by the pager's offset check alone.
        self.for_each_plex_page(connection, url, auth, |entries| {
            rows += entries.len();
            skipped += fold_plex_history_into(&entries, &mut plays);
            Ok(if plays.len() >= SIGNAL_MAX_ITEMS {
                Some(SIGNAL_MAX_ITEMS)
            } else if rows >= SIGNAL_MAX_HISTORY_ROWS {
                Some(SIGNAL_MAX_HISTORY_ROWS)
            } else {
                None
            })
        })
        .await?;
        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped, "skipped unreadable Plex history entries"
            );
            return Err(unreadable_items());
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
            return Err(unreadable_items());
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

    /// Hand every page of a paged Plex listing to `consume`, failing as
    /// incomplete when the listing cannot be read whole.
    ///
    /// `consume` answers with the cap the participant has now reached, if
    /// any; that fails the read only if another page remains.
    ///
    /// Every page must echo the `offset` it was asked for whenever the
    /// listing is longer than that page, so a server that ignores the paging
    /// headers cannot pass by answering every request from the top.
    async fn for_each_plex_page(
        &self,
        connection: &MediaServerConnection,
        url: Url,
        auth: SignalAuth<'_>,
        mut consume: impl FnMut(Vec<Value>) -> ReadResult<Option<usize>>,
    ) -> ReadResult<()> {
        let mut start = 0_usize;
        loop {
            let page = PlexPage {
                start,
                size: SIGNAL_PAGE_SIZE,
            };
            let body = self.fetch_json(url.clone(), auth, Some(page)).await?;
            let container = plex_container(&body)?;
            let total = container.get("totalSize").and_then(integer_field);
            let page_entries = match container.get("Metadata") {
                None => Vec::new(),
                Some(value) => value.as_array().cloned().ok_or_else(unreadable_listing)?,
            };
            let page_len = page_entries.len();
            // A server that answers a later page from the top again would
            // otherwise loop or duplicate; neither is a complete read. Without
            // an offset there is no telling, unless this page is the whole
            // listing.
            match container.get("offset").and_then(integer_field) {
                Some(offset) if offset != start as i64 => return Err(paging_not_honoured()),
                None if total.is_some_and(|total| total > page_len as i64) => {
                    return Err(paging_not_honoured());
                }
                _ => {}
            }
            let cap = consume(page_entries)?;
            start += page_len;

            if !another_page_remains(start, page_len, total)? {
                return Ok(());
            }
            if let Some(cap) = cap {
                return Err(item_cap_reached(connection, cap));
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
    ) -> ReadResult<Vec<Value>> {
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
                        Err(failure) => return Err(failure.into_failure()),
                    }
                }
                Ok(entries)
            }
            Err(FetchFailure::Status(StatusCode::NOT_FOUND)) => Ok(Vec::new()),
            Err(failure) => Err(failure.into_failure()),
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
    async fn open(
        &self,
        connection: &MediaServerConnection,
    ) -> Result<Box<dyn MediaServerSignalReader>, SignalReadFailure> {
        let token = connection
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                SignalReadFailure::new(reasons::CREDENTIAL_MISSING, "no credential stored")
            })?
            .to_string();
        let base = server_base(&connection.base_url)?;

        match connection.provider {
            MediaServerProvider::Jellyfin | MediaServerProvider::Emby => {
                Ok(Box::new(UserItemsReader {
                    source: self.clone(),
                    connection: connection.clone(),
                    token,
                    base,
                }))
            }
            MediaServerProvider::Plex => {
                let base = self.plex_server(connection, base, &token).await?;
                let owner_ids = self.plex_owner_account_ids(&token).await?;
                Ok(Box::new(PlexReader {
                    source: self.clone(),
                    connection: connection.clone(),
                    owner_token: token,
                    base,
                    owner_ids,
                    shared_tokens: OnceCell::new(),
                }))
            }
        }
    }
}

/// A participant's id as given, or the refusal for one that is blank.
fn participant_id(external_user_id: &str) -> ReadResult<&str> {
    let id = external_user_id.trim();
    if id.is_empty() {
        return Err(SignalReadFailure::new(
            reasons::PARTICIPANT_ID_MISSING,
            "a media-server signal read needs a provider user id",
        ));
    }
    Ok(id)
}

/// An opened Jellyfin or Emby connection.
struct UserItemsReader {
    source: HttpMediaServerSignalSource,
    connection: MediaServerConnection,
    token: String,
    base: Url,
}

#[async_trait]
impl MediaServerSignalReader for UserItemsReader {
    async fn fetch_played_items(
        &self,
        external_user_id: &str,
    ) -> Result<Vec<ProviderPlayedItem>, SignalReadFailure> {
        let external_user_id = participant_id(external_user_id)?;
        let (auth, fields) = match self.connection.provider {
            MediaServerProvider::Emby => (
                SignalAuth::Emby {
                    token: &self.token,
                    connection_id: &self.connection.id,
                },
                EMBY_ITEM_FIELDS,
            ),
            _ => (SignalAuth::Jellyfin(&self.token), JELLYFIN_ITEM_FIELDS),
        };
        self.source
            .fetch_user_items(&self.connection, &self.base, auth, fields, external_user_id)
            .await
    }
}

/// An opened Plex connection: the server's address and the owner's account
/// ids, resolved once for every participant.
struct PlexReader {
    source: HttpMediaServerSignalSource,
    connection: MediaServerConnection,
    owner_token: String,
    base: Url,
    owner_ids: Vec<String>,
    /// Shared users' server tokens, read from plex.tv at most once per
    /// connection and only if a non-owner participant needs one. `None` when
    /// they could not be listed.
    shared_tokens: OnceCell<Option<BTreeMap<String, String>>>,
}

impl PlexReader {
    /// The server access token of a user the server is shared with, if plex.tv
    /// lists one for them.
    async fn shared_token(&self, account_id: &str) -> Option<String> {
        let tokens = self
            .shared_tokens
            .get_or_init(|| async {
                let machine_id = self
                    .connection
                    .machine_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())?;
                match self
                    .source
                    .plex_shared_server_tokens(machine_id, &self.owner_token)
                    .await
                {
                    Ok(tokens) => Some(tokens),
                    Err(failure) => {
                        let failure = failure.into_failure();
                        warn!(
                            connection_id = self.connection.id.as_str(),
                            reason = failure.reason,
                            error = %failure,
                            "could not list Plex shared-server tokens; shared users are read from history"
                        );
                        None
                    }
                }
            })
            .await;
        tokens.as_ref()?.get(account_id).cloned()
    }
}

#[async_trait]
impl MediaServerSignalReader for PlexReader {
    async fn fetch_played_items(
        &self,
        external_user_id: &str,
    ) -> Result<Vec<ProviderPlayedItem>, SignalReadFailure> {
        let external_user_id = participant_id(external_user_id)?;
        let source = &self.source;
        let owner_auth = SignalAuth::Plex(&self.owner_token);

        if self.owner_ids.iter().any(|id| id == external_user_id) {
            let mut items = source
                .fetch_plex_watched_set(&self.connection, &self.base, owner_auth)
                .await?;
            source
                .attach_plex_series_ids(&self.base, owner_auth, &mut items)
                .await;
            return Ok(items);
        }

        // Shared users and history both key on Plex's numeric account id.
        // Anything else would match nobody and read back as a clean, empty
        // history.
        if !external_user_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(SignalReadFailure::new(
                reasons::PLEX_ACCOUNT_ID_INVALID,
                "Plex account id is not a server account id",
            ));
        }

        // A user the server is shared with is read like the owner, with their
        // own token; anyone plex.tv lists no token for is read from history.
        let shared_token = self.shared_token(external_user_id).await;
        let (mut items, auth) = match shared_token.as_deref() {
            Some(token) => {
                let auth = SignalAuth::Plex(token);
                let items = source
                    .fetch_plex_watched_set(&self.connection, &self.base, auth)
                    .await?;
                (items, auth)
            }
            None => {
                let items = source
                    .fetch_plex_history(&self.connection, &self.base, owner_auth, external_user_id)
                    .await?;
                (items, owner_auth)
            }
        };
        source
            .attach_plex_series_ids(&self.base, auth, &mut items)
            .await;
        Ok(items)
    }
}

/// `userID` → `accessToken` from plex.tv's shared-server list. `None` when the
/// body is not readable XML.
fn shared_server_tokens(xml: &str) -> Option<BTreeMap<String, String>> {
    let mut reader = Reader::from_str(xml);
    let mut tokens = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element) | Event::Empty(element)) => {
                let mut user_id = None;
                let mut access_token = None;
                for attribute in element.attributes().flatten() {
                    let Ok(value) = attribute.normalized_value(XmlVersion::Implicit1_0) else {
                        continue;
                    };
                    let value = value.trim().to_string();
                    match attribute.key.as_ref() {
                        "userID" => user_id = Some(value),
                        "accessToken" => access_token = Some(value),
                        _ => {}
                    }
                }
                if let (Some(user_id), Some(access_token)) = (user_id, access_token)
                    && !user_id.is_empty()
                    && !access_token.is_empty()
                {
                    tokens.insert(user_id, access_token);
                }
            }
            Ok(Event::Eof) => return Some(tokens),
            Ok(_) => {}
            Err(_) => return None,
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
        file_paths: string_field(entry, "Path").into_iter().collect(),
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
        // records what the server said, and a row with no views is unplayed.
        played: view_count.unwrap_or(0) > 0,
        play_count: view_count.unwrap_or(0).max(0),
        last_played_at: entry.get("lastViewedAt").and_then(epoch_timestamp),
        // Plex has no favourite.
        is_favorite: false,
        file_paths: plex_file_paths(entry),
    })
}

/// Every `Media[].Part[].file` of a Plex item: one per file of each version.
fn plex_file_paths(entry: &Value) -> Vec<String> {
    entry
        .get("Media")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|media| media.get("Part").and_then(Value::as_array))
        .flatten()
        .filter_map(|part| part.get("file").and_then(Value::as_str))
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .map(str::to_string)
        .collect()
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
#[cfg(test)]
fn fold_plex_history(entries: &[Value]) -> (BTreeMap<String, PlexPlays>, usize) {
    let mut plays = BTreeMap::new();
    let skipped = fold_plex_history_into(entries, &mut plays);
    (plays, skipped)
}

/// Fold one page of history entries into `plays`. Returns how many were
/// unreadable.
fn fold_plex_history_into(entries: &[Value], plays: &mut BTreeMap<String, PlexPlays>) -> usize {
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
    skipped
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
fn server_base(base_url: &str) -> ReadResult<Url> {
    let mut base = Url::parse(base_url.trim()).map_err(|_| invalid_base_url())?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base)
}

fn invalid_base_url() -> SignalReadFailure {
    SignalReadFailure::new(
        reasons::BASE_URL_INVALID,
        "media server base URL is invalid",
    )
}

fn played_items_url(
    base: &Url,
    external_user_id: &str,
    start_index: usize,
    fields: &str,
) -> ReadResult<Url> {
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
fn series_lookup_url(base: &Url, external_user_id: &str, ids: &[String]) -> ReadResult<Url> {
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

fn plex_url(base: &Url, path: &str) -> ReadResult<Url> {
    base.join(path).map_err(|_| invalid_base_url())
}

/// `/library/metadata/{k1,k2,…}` with the `Guid` list switched on.
fn plex_metadata_url(base: &Url, keys: &[String]) -> ReadResult<Url> {
    let mut url = plex_url(base, &format!("library/metadata/{}", keys.join(",")))?;
    url.query_pairs_mut().append_pair("includeGuids", "1");
    Ok(url)
}

/// The `MediaContainer` every Plex answer is wrapped in. Without one the body
/// is not a Plex answer at all, and reading it as "nothing" would be a guess.
fn plex_container(body: &Value) -> ReadResult<&Value> {
    body.get("MediaContainer")
        .filter(|container| container.is_object())
        .ok_or_else(unreadable_listing)
}

fn plex_metadata_entries(body: &Value) -> ReadResult<Vec<Value>> {
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
    fn into_failure(self) -> SignalReadFailure {
        match self {
            Self::Status(status @ (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)) => {
                SignalReadFailure::new(
                    reasons::ACCESS_DENIED,
                    format!("status {}", status.as_u16()),
                )
            }
            Self::Status(status) => SignalReadFailure::new(
                reasons::REQUEST_REJECTED,
                format!("status {}", status.as_u16()),
            ),
            Self::Transport(reason) => SignalReadFailure::new(reasons::REQUEST_FAILED, reason),
            Self::Unreadable => {
                SignalReadFailure::new(reasons::RESPONSE_UNREADABLE, "unreadable response")
            }
        }
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
    use wiremock::{Request, ResponseTemplate};

    /// Open the connection and read one participant, as a sweep does.
    #[async_trait]
    trait ReadPlayed {
        async fn read_played(
            &self,
            connection: &MediaServerConnection,
            external_user_id: &str,
        ) -> ReadResult<Vec<ProviderPlayedItem>>;
    }

    #[async_trait]
    impl ReadPlayed for HttpMediaServerSignalSource {
        async fn read_played(
            &self,
            connection: &MediaServerConnection,
            external_user_id: &str,
        ) -> ReadResult<Vec<ProviderPlayedItem>> {
            self.open(connection)
                .await?
                .fetch_played_items(external_user_id)
                .await
        }
    }

    /// A paged listing generated from the start each request asks for: Plex
    /// sends it as a container header, Jellyfin and Emby as `StartIndex`.
    /// `page_limit` models a server that clamps the page size it was asked
    /// for; `distinct_keys` folds Plex history rows onto that many items.
    struct GeneratedPages {
        plex: bool,
        total: usize,
        report_total: bool,
        page_limit: usize,
        distinct_keys: Option<usize>,
    }

    impl wiremock::Respond for GeneratedPages {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let start = if self.plex {
                request
                    .headers
                    .get("X-Plex-Container-Start")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<usize>().ok())
            } else {
                request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "StartIndex")
                    .and_then(|(_, value)| value.parse::<usize>().ok())
            }
            .unwrap_or(0);
            let end = start.saturating_add(self.page_limit).min(self.total);
            let rows = (start..end.max(start))
                .map(|index| {
                    if self.plex {
                        let key = self.distinct_keys.map_or(index, |keys| index % keys);
                        json!({
                            "ratingKey": key.to_string(),
                            "type": "movie",
                            "viewedAt": 1_700_000_000 + index as i64
                        })
                    } else {
                        json!({
                            "Id": format!("jf-generated-{index}"),
                            "Type": "Movie",
                            "ProviderIds": { "Tmdb": index.to_string() },
                            "UserData": { "Played": true, "PlayCount": 1 }
                        })
                    }
                })
                .collect::<Vec<_>>();
            let body = if self.plex {
                let mut container =
                    json!({ "size": rows.len(), "offset": start, "Metadata": rows });
                if self.report_total {
                    container["totalSize"] = json!(self.total);
                }
                json!({ "MediaContainer": container })
            } else {
                let mut body = json!({ "Items": rows });
                if self.report_total {
                    body["TotalRecordCount"] = json!(self.total);
                }
                body
            };
            ResponseTemplate::new(200).set_body_json(body)
        }
    }

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
            .read_played(&connection, "user-123")
            .await
            .expect_err("no credential");
        assert_eq!(error.reason, reasons::CREDENTIAL_MISSING);
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
            .read_played(&connection, "user-123")
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
            .read_played(&connection, "user-123")
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
            .read_played(&connection, "user-123")
            .await
            .expect_err("401 fails the read");
        assert_eq!(error.reason, reasons::ACCESS_DENIED);
        let message = format!("{error}");
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
                .read_played(&connection, "user-123")
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
            .and(query_param("Fields", "ProviderIds,Path"))
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
            .read_played(
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
            .read_played(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("an unreadable item makes the read incomplete");
        assert_eq!(error.reason, reasons::ITEMS_UNREADABLE);

        // A server that always has another full page of new items.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(GeneratedPages {
                plex: false,
                total: usize::MAX,
                report_total: false,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
            })
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .read_played(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("the cap makes the read incomplete");
        assert_eq!(error.reason, reasons::ITEM_CAP_REACHED);

        // A full first page, then a refused second one.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", "0"))
            .respond_with(GeneratedPages {
                plex: false,
                total: usize::MAX,
                report_total: false,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", SIGNAL_PAGE_SIZE.to_string()))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .read_played(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("a refused page fails the read");
        assert_eq!(error.reason, reasons::REQUEST_REJECTED);
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
            .read_played(&plex_connection(server.uri()), PLEX_OWNER_ID)
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

    /// A user plex.tv lists no server token for is read from server history by
    /// their own account id.
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
        // plex.tv lists no server token for this user (a managed home user
        // has none), so history is the only way to read them.
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer size="1"><SharedServer userID="4004" accessToken="other-token" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
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
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
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

        async fn history_error(body: impl wiremock::Respond + 'static) -> SignalReadFailure {
            let server = MockServer::start().await;
            mount_plex_owner(&server).await;
            Mock::given(method("GET"))
                .and(path("/status/sessions/history/all"))
                .respond_with(body)
                .mount(&server)
                .await;
            let error = plex_source(&server)
                .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
                .await
                .expect_err("a partial history is not a complete one");
            error
        }

        // An entry with no rating key.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": 1, "Metadata": [
                { "type": "movie", "viewedAt": 1_700_000_000 }
            ] }
        })))
        .await;
        assert_eq!(error.reason, reasons::ITEMS_UNREADABLE);

        // A server that always has another full page of new items.
        let error = history_error(GeneratedPages {
            plex: true,
            total: usize::MAX,
            report_total: false,
            page_limit: SIGNAL_PAGE_SIZE,
            distinct_keys: None,
        })
        .await;
        assert_eq!(error.reason, reasons::ITEM_CAP_REACHED);

        // A server that stops short of the total it reported.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": 0, "offset": 0, "totalSize": 3, "Metadata": [] }
        })))
        .await;
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);

        let full_page = (0..SIGNAL_PAGE_SIZE)
            .map(|index| json!({ "ratingKey": index.to_string(), "type": "movie", "viewedAt": 1_700_000_000 }))
            .collect::<Vec<_>>();

        // A server that ignores the paging headers and answers from the top.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": SIGNAL_PAGE_SIZE, "offset": 0, "Metadata": full_page }
        })))
        .await;
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);

        // A body that is not a Plex answer.
        let error =
            history_error(ResponseTemplate::new(200).set_body_json(json!({ "Unexpected": [] })))
                .await;
        assert_eq!(error.reason, reasons::RESPONSE_UNREADABLE);

        // A refused request.
        let error = history_error(ResponseTemplate::new(401)).await;
        assert_eq!(error.reason, reasons::ACCESS_DENIED);
        assert!(!format!("{error}").contains(PLEX_TOKEN), "{error}");

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
            .read_played(&plex_connection(server.uri()), PLEX_OWNER_ID)
            .await
            .expect_err("an unreadable library item makes the read incomplete");
        assert_eq!(error.reason, reasons::ITEMS_UNREADABLE);
    }

    /// A shared user whose linked id is not a server account id would match
    /// nobody in history; that is an error, never an empty history. So is a
    /// plex.tv connection with no server selected.
    #[tokio::test]
    async fn plex_reads_that_could_only_come_back_empty_are_errors() {
        use wiremock::MockServer;

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        let error = plex_source(&server)
            .read_played(&plex_connection(server.uri()), "plex-other-uuid")
            .await
            .expect_err("a non-numeric account id cannot filter history");
        assert_eq!(error.reason, reasons::PLEX_ACCOUNT_ID_INVALID);

        let mut through_service = plex_connection("https://plex.tv".into());
        through_service.machine_id = None;
        let error = plex_source(&server)
            .read_played(&through_service, PLEX_SHARED_ID)
            .await
            .expect_err("plex.tv names no server without a machine id");
        assert_eq!(error.reason, reasons::PLEX_SERVER_NOT_SELECTED);
    }

    // ── Paging and bounds ───────────────────────────────────────────────────

    /// A server that hands back fewer rows than asked for is asked for the
    /// next page until its own total is reached, not taken to have finished.
    #[tokio::test]
    async fn a_server_that_clamps_its_page_size_is_paged_to_its_reported_total() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(GeneratedPages {
                plex: false,
                total: 250,
                report_total: true,
                page_limit: 100,
                distinct_keys: None,
            })
            .expect(3)
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .read_played(&connection, "user-123")
            .await
            .expect("every clamped page is read");
        assert_eq!(items.len(), 250);
    }

    #[tokio::test]
    async fn an_empty_page_before_the_reported_total_is_incomplete() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Items": [], "TotalRecordCount": 3 })),
            )
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let error = HttpMediaServerSignalSource::new()
            .read_played(&connection, "user-123")
            .await
            .expect_err("an empty page short of the total is not the end");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// The cap fails a read only when another page remains: a history of
    /// exactly the cap, with the server's total saying so, is complete.
    #[tokio::test]
    async fn a_history_of_exactly_the_cap_is_complete() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(GeneratedPages {
                plex: false,
                total: SIGNAL_MAX_ITEMS,
                report_total: true,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
            })
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .read_played(&connection, "user-123")
            .await
            .expect("a complete read at the cap is not truncated");
        assert_eq!(items.len(), SIGNAL_MAX_ITEMS);
    }

    /// Plex history has a row per play. A heavy rewatcher with more plays
    /// than the item cap, over few items, is still read whole.
    #[tokio::test]
    async fn plex_history_caps_distinct_items_not_plays() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(GeneratedPages {
                plex: true,
                total: SIGNAL_MAX_ITEMS + 5_000,
                report_total: true,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: Some(2),
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/0,1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 2, "Metadata": [
                    { "ratingKey": "0", "type": "movie", "Guid": [{ "id": "tmdb://11" }] },
                    { "ratingKey": "1", "type": "movie", "Guid": [{ "id": "tmdb://12" }] }
                ] }
            })))
            .mount(&server)
            .await;

        let items = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect("plays beyond the item cap over two items are a complete read");
        assert_eq!(items.len(), 2);
        let plays = (SIGNAL_MAX_ITEMS + 5_000) as i64 / 2;
        assert!(items.iter().all(|item| item.play_count == plays));
    }

    /// A watched-set row with no views means `viewCount>>=0` was not applied;
    /// the listing is then not a played set and the read fails.
    #[tokio::test]
    async fn a_plex_watched_set_with_unwatched_rows_is_incomplete() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let unwatched = normalize_plex_item(&plex_movie(0, 0)).expect("normalizes");
        assert!(!unwatched.played);
        let mut no_count = plex_movie(1, 0);
        no_count
            .as_object_mut()
            .expect("object")
            .remove("viewCount");
        assert!(!normalize_plex_item(&no_count).expect("normalizes").played);

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
                "MediaContainer": { "size": 2, "offset": 0, "totalSize": 2, "Metadata": [
                    plex_movie(1, 1_700_000_000),
                    { "ratingKey": "102", "type": "movie", "viewCount": 0 }
                ] }
            })))
            .mount(&server)
            .await;

        let error = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_OWNER_ID)
            .await
            .expect_err("an unfiltered listing is not a played set");
        assert_eq!(error.reason, reasons::FILTER_NOT_HONOURED);
    }

    // ── Plex server selection and shared users ──────────────────────────────

    fn plex_resources(machine_id: &str, uri: &str) -> String {
        format!(
            r#"<MediaContainer size="1"><Device name="Example Server" provides="server" clientIdentifier="{machine_id}"><Connection protocol="http" address="127.0.0.1" port="32400" uri="{uri}" local="1" /></Device></MediaContainer>"#
        )
    }

    async fn mount_owner_library(server: &wiremock::MockServer, token: &str, view_count: i64) {
        use wiremock::Mock;
        use wiremock::matchers::{header, method, path};
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .and(header("X-Plex-Token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "Directory": [{ "key": "1", "type": "movie" }] }
            })))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(header("X-Plex-Token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "offset": 0, "totalSize": 1,
                    "Metadata": [plex_movie(view_count, 1_756_588_445)] }
            })))
            .mount(server)
            .await;
    }

    /// The standard Plex connection is plex.tv plus a machine id. The server's
    /// own address comes from the account's resource list, once per opening.
    #[tokio::test]
    async fn a_plex_tv_connection_reads_the_server_its_machine_id_selects() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        // One mock plays plex.tv; the resource list points back at it as the
        // media server.
        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_owner_library(&server, PLEX_TOKEN, 3).await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(plex_resources("plex-machine-1", &server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let connection = plex_connection("https://plex.tv".into());
        let items = plex_source(&server)
            .read_played(&connection, PLEX_OWNER_ID)
            .await
            .expect("the selected server is read");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].play_count, 3);

        // A machine id the account does not list leaves nothing to read.
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(plex_resources("plex-machine-other", &other.uri())),
            )
            .mount(&other)
            .await;
        let error = plex_source(&other)
            .read_played(&connection, PLEX_OWNER_ID)
            .await
            .expect_err("an unlisted server cannot be reached");
        assert_eq!(error.reason, reasons::PLEX_SERVER_UNREACHABLE);
    }

    /// One opening serves every participant: the owner's account and the
    /// shared-server tokens are each read once. A user the server is shared
    /// with is read from the library with their own token, so their own view
    /// count is recorded; a user plex.tv lists no token for is read from
    /// history.
    #[tokio::test]
    async fn a_plex_connection_reads_shared_users_with_their_own_tokens() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer};

        const SHARED_TOKEN: &str = "shared-token-2002";
        const MANAGED_ID: &str = "3003";

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": 1001, "uuid": "plex-owner-uuid" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"<MediaContainer machineIdentifier="plex-machine-1" size="1"><SharedServer id="7" username="example-shared" userID="{PLEX_SHARED_ID}" accessToken="{SHARED_TOKEN}" owned="0"><Section id="1" key="1" title="Movies" type="movie" shared="1" /></SharedServer></MediaContainer>"#
            )))
            .expect(1)
            .mount(&server)
            .await;
        mount_owner_library(&server, PLEX_TOKEN, 9).await;
        mount_owner_library(&server, SHARED_TOKEN, 2).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("accountID", PLEX_SHARED_ID))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("accountID", MANAGED_ID))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0, "offset": 0, "totalSize": 0 }
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(plex_resources("plex-machine-1", &server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let reader = plex_source(&server)
            .open(&plex_connection("https://plex.tv".into()))
            .await
            .expect("opens");
        let owner = reader
            .fetch_played_items(PLEX_OWNER_ID)
            .await
            .expect("owner");
        let shared = reader
            .fetch_played_items(PLEX_SHARED_ID)
            .await
            .expect("shared user");
        let managed = reader
            .fetch_played_items(MANAGED_ID)
            .await
            .expect("managed user");

        assert_eq!(owner[0].play_count, 9);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].play_count, 2, "the shared user's own view count");
        assert!(managed.is_empty());
    }

    // ── Paging that is not honoured ─────────────────────────────────────────

    /// A server that ignores `StartIndex` but whose repeated page stays inside
    /// the total it reports would otherwise read as complete.
    #[tokio::test]
    async fn a_jellyfin_server_that_repeats_its_first_page_is_not_honouring_paging() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer};

        // Every request starts the listing again, under a total of two pages.
        let repeating = MockServer::start().await;
        let first_page = (0..SIGNAL_PAGE_SIZE)
            .map(|index| {
                json!({
                    "Id": format!("jf-generated-{index}"),
                    "Type": "Movie",
                    "ProviderIds": { "Tmdb": index.to_string() },
                    "UserData": { "Played": true, "PlayCount": 1 }
                })
            })
            .collect::<Vec<_>>();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": first_page,
                "TotalRecordCount": SIGNAL_PAGE_SIZE * 2
            })))
            .mount(&repeating)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .read_played(&emby_connection(repeating.uri()), "emby-user-1")
            .await
            .expect_err("a repeated page is not the rest of the listing");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// Jellyfin echoes the start it served; a page that says it started
    /// somewhere else is not the page that was asked for.
    #[tokio::test]
    async fn a_jellyfin_page_that_echoes_another_start_is_not_honouring_paging() {
        use wiremock::matchers::{method, query_param};
        use wiremock::{Mock, MockServer};

        let rows = |from: usize| {
            (from..from + SIGNAL_PAGE_SIZE)
                .map(|index| {
                    json!({
                        "Id": format!("jf-generated-{index}"),
                        "Type": "Movie",
                        "ProviderIds": { "Tmdb": index.to_string() },
                        "UserData": { "Played": true, "PlayCount": 1 }
                    })
                })
                .collect::<Vec<_>>()
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": rows(0),
                "StartIndex": 0,
                "TotalRecordCount": SIGNAL_PAGE_SIZE * 2
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("StartIndex", SIGNAL_PAGE_SIZE.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": rows(SIGNAL_PAGE_SIZE),
                "StartIndex": 0,
                "TotalRecordCount": SIGNAL_PAGE_SIZE * 2
            })))
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .read_played(
                &MediaServerConnection {
                    base_url: server.uri(),
                    ..connection_fixture()
                },
                "jf-user-1",
            )
            .await
            .expect_err("a page from the wrong start is not the next page");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// A Plex watched set whose second page repeats the first one's items,
    /// even under a correct `offset` and total, is not the rest of the set.
    #[tokio::test]
    async fn a_plex_watched_set_that_repeats_items_is_not_honouring_paging() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        let page = |offset: usize| {
            let rows = (0..SIGNAL_PAGE_SIZE)
                .map(|index| {
                    json!({
                        "ratingKey": index.to_string(),
                        "type": "movie",
                        "viewCount": 1,
                        "lastViewedAt": 1_700_000_000
                    })
                })
                .collect::<Vec<_>>();
            json!({ "MediaContainer": {
                "size": SIGNAL_PAGE_SIZE,
                "offset": offset,
                "totalSize": SIGNAL_PAGE_SIZE * 2,
                "Metadata": rows
            } })
        };
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
            .and(header("X-Plex-Container-Start", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(0)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(header(
                "X-Plex-Container-Start",
                SIGNAL_PAGE_SIZE.to_string().as_str(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(SIGNAL_PAGE_SIZE)))
            .mount(&server)
            .await;

        let error = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_OWNER_ID)
            .await
            .expect_err("repeated items are not the next page");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// Plex history rows repeat items by design, so a history page that is
    /// one of several must say which offset it starts at.
    #[tokio::test]
    async fn a_plex_history_page_without_an_offset_is_not_honouring_paging() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let rows = (0..SIGNAL_PAGE_SIZE)
            .map(|index| {
                json!({
                    "ratingKey": (index % 3).to_string(),
                    "type": "movie",
                    "viewedAt": 1_700_000_000 + index as i64
                })
            })
            .collect::<Vec<_>>();
        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": SIGNAL_PAGE_SIZE,
                    "totalSize": SIGNAL_PAGE_SIZE * 2,
                    "Metadata": rows
                }
            })))
            .mount(&server)
            .await;

        let error = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect_err("a page that cannot be placed is not proof of completeness");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    // ── File paths ──────────────────────────────────────────────────────────

    /// The server's file paths are carried for the foreign check only; every
    /// part of a Plex item counts.
    #[test]
    fn reads_the_servers_file_paths_for_each_provider() {
        let mut jellyfin = movie_item();
        jellyfin["Path"] = json!("/mnt/server-library/Example Feature/feature.mkv");
        assert_eq!(
            normalize_jellyfin_item(&jellyfin)
                .expect("normalizes")
                .file_paths,
            vec!["/mnt/server-library/Example Feature/feature.mkv".to_string()]
        );
        assert!(
            normalize_jellyfin_item(&emby_movie_item())
                .expect("normalizes")
                .file_paths
                .is_empty()
        );

        let mut plex = plex_movie(1, 1_700_000_000);
        plex["Media"] = json!([
            { "Part": [
                { "file": "/mnt/server-library/Example Feature/part-1.mkv" },
                { "file": "/mnt/server-library/Example Feature/part-2.mkv" }
            ] },
            { "Part": [{ "file": "/mnt/server-library/Example Feature/alt.mkv" }] }
        ]);
        assert_eq!(
            normalize_plex_item(&plex).expect("normalizes").file_paths,
            vec![
                "/mnt/server-library/Example Feature/part-1.mkv".to_string(),
                "/mnt/server-library/Example Feature/part-2.mkv".to_string(),
                "/mnt/server-library/Example Feature/alt.mkv".to_string(),
            ]
        );
        assert!(
            normalize_plex_item(&plex_movie(1, 1_700_000_000))
                .expect("normalizes")
                .file_paths
                .is_empty()
        );
    }

    #[test]
    fn reads_shared_server_tokens_by_account_id() {
        let tokens = shared_server_tokens(
            r#"<MediaContainer><SharedServer userID="2002" accessToken="token-a" /><SharedServer userID="3003" accessToken="" /><Section key="1" /></MediaContainer>"#,
        )
        .expect("readable");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens.get("2002").map(String::as_str), Some("token-a"));
        assert!(shared_server_tokens("<MediaContainer><SharedServer").is_none());
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
