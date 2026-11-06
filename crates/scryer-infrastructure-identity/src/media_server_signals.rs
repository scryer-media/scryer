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
//! design, one row per play, so it is placed by plays instead: the same
//! (item, time, account) seen twice is a repeated page, and an `offset` is
//! checked only when the server sends one. Each fails as a paging failure.
//!
//! A **page size** keeps a large library from arriving as one enormous body.
//! There is no cap on how much one participant has watched: a read goes on
//! until the listing is whole, and Plex history items are looked up page by
//! page so the rows themselves are never held. What bounds a read is the
//! server's own total, as first reported: a listing read a full page past it
//! is not paging as asked (a total that keeps climbing, or a server that keeps
//! answering with new rows), and fails as a paging failure.
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
//! * **anyone else** (a managed home user, who has no shared-server entry) is
//!   read from `/status/sessions/history/all` filtered by `accountID`. When
//!   plex.tv cannot list the tokens, no one but the owner is read: history
//!   keeps only organic plays, so it is not a stand-in for a shared user's
//!   own library view. Every history row must name the account asked for, or
//!   the filter was not applied and the read fails. Plays are folded
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
//! movie nor an episode, makes the read incomplete, as
//! `WATCH_SYNC_ITEMS_UNREADABLE`; so does a history row with no account, no
//! item or no type, and an item whose metadata cannot be read.
//! `WATCH_SYNC_FILTER_NOT_HONOURED` is kept for a listing whose filter was
//! visibly ignored: an unplayed row in a watched-set listing, or a filtered
//! history row naming another account or item. Publishing a partial history
//! could turn an omitted played item into a false "never watched"
//! maintenance fact.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{StreamExt, TryStreamExt};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use reqwest::StatusCode;
use scryer_application::media_server_signals::reasons;
use scryer_application::{
    MediaServerSignalReader, MediaServerSignalSource, ProviderAudiencePlay, ProviderItemPlay,
    ProviderItemRecheck, ProviderPlayedItem, SignalReadFailure,
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

/// Series and item metadata lookups are batched; this bounds one batch's id
/// list so the request line stays a sane length.
const LOOKUP_BATCH: usize = 100;

/// The most pages one listing may take before the read gives up on it.
///
/// Every pager otherwise ends only when the server says so: its reported
/// total is reached, or, when it reports none, a page comes back short. A
/// server that reports no total and keeps answering full pages would never
/// end, so a listing that reaches this many pages without the server
/// signalling its end fails as [`reasons::PAGING_NOT_HONOURED`]. At
/// [`SIGNAL_PAGE_SIZE`] rows a page this is two million rows, far past any
/// real watch history, so only a server that is not paging as asked meets it.
const SIGNAL_PAGE_CEILING: usize = 4_000;

/// How many single-item requests a live re-check keeps in flight where the
/// provider cannot answer for several items at once (Plex history is read per
/// item). Batched reads need no bound: their count is already the item count
/// over [`LOOKUP_BATCH`].
const RECHECK_CONCURRENCY: usize = 4;

/// The account id Plex server history gives the server's owner: the server's
/// own local account, never the owner's plex.tv id.
const PLEX_LOCAL_OWNER_ACCOUNT: &str = "1";

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

/// A paged Plex listing, read one page at a time so a caller can do its own
/// reads between pages.
///
/// An `offset` that differs from the start asked for always fails. With
/// `offset_required`, every page must also echo one whenever the listing is
/// longer than that page, so a server that ignores the paging headers cannot
/// pass by answering every request from the top; a caller that passes `false`
/// must catch a repeated page itself.
struct PlexPager {
    url: Url,
    offset_required: bool,
    start: usize,
    pages: usize,
    first_total: Option<i64>,
    last_total: Option<i64>,
    done: bool,
}

impl PlexPager {
    fn new(url: Url, offset_required: bool) -> Self {
        Self {
            url,
            offset_required,
            start: 0,
            pages: 0,
            first_total: None,
            last_total: None,
            done: false,
        }
    }

    /// The next page's rows, or `None` once the listing is whole.
    async fn next_page(
        &mut self,
        source: &HttpMediaServerSignalSource,
        auth: SignalAuth<'_>,
    ) -> ReadResult<Option<Vec<Value>>> {
        if self.done {
            return Ok(None);
        }
        let page = PlexPage {
            start: self.start,
            size: SIGNAL_PAGE_SIZE,
        };
        let body = source
            .fetch_json(self.url.clone(), auth, Some(page))
            .await?;
        let container = plex_container(&body)?;
        let total = container.get("totalSize").and_then(integer_field);
        let entries = match container.get("Metadata") {
            None => Vec::new(),
            Some(value) => value.as_array().cloned().ok_or_else(unreadable_listing)?,
        };
        let page_len = entries.len();
        // A server that answers a later page from the top again would
        // otherwise loop or duplicate; neither is a complete read. Without an
        // offset there is no telling, unless this page is the whole listing.
        match container.get("offset").and_then(integer_field) {
            Some(offset) if offset != self.start as i64 => return Err(paging_not_honoured()),
            None if self.offset_required && total.is_some_and(|total| total > page_len as i64) => {
                return Err(paging_not_honoured());
            }
            _ => {}
        }
        self.start += page_len;
        self.first_total = self.first_total.or(total);
        self.last_total = total.or(self.last_total);
        if past_reported_total(self.start, self.first_total) {
            return Err(paging_not_honoured());
        }
        self.done = !another_page_remains(self.start, page_len, total)?;
        self.pages += 1;
        if !self.done && self.pages >= source.page_ceiling {
            return Err(page_ceiling_reached());
        }
        Ok(Some(entries))
    }
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

/// Whether `read` rows have gone a full page past the total the listing first
/// reported. A total may grow a little while a sweep reads, as new plays land;
/// a whole page beyond it means the server is not paging as asked, and without
/// this a listing whose total keeps climbing would never end.
fn past_reported_total(read: usize, first_total: Option<i64>) -> bool {
    first_total.is_some_and(|total| read as i64 >= total.max(0) + SIGNAL_PAGE_SIZE as i64)
}

/// One Plex history row as a play: its item, time and account. Rows repeat an
/// item by design, one per play, but never one play. History is read oldest
/// first, so a play landing mid-read joins the end of the listing. A row
/// inserted before the cursor shifts later rows forward, and the row the next
/// page then repeats is skipped as already seen; a row removed before the
/// cursor shifts later rows back, and the next page silently passes one over.
/// Either way fewer distinct rows are read than the listing last reported,
/// which [`history_read_short_of_total`] fails. A page made only of rows
/// already seen was answered from elsewhere, and fails too. Hashed, so a long
/// history's placement costs eight bytes a row; a collision can only skip a
/// row, which the distinct count then fails as paging, never invent a play.
fn plex_play_fingerprint(entry: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    for key in ["ratingKey", "viewedAt", "accountID"] {
        entry.get(key).and_then(scalar_string).hash(&mut hasher);
    }
    hasher.finish()
}

/// Whether a whole history read came up with fewer distinct plays than the
/// listing last reported. Rows shifted by a play that came or went before the
/// cursor, or two plays sharing a time that swap places between requests,
/// leave a play unread while every page still looks well formed.
fn history_read_short_of_total(distinct: usize, last_total: Option<i64>) -> bool {
    last_total.is_some_and(|total| (distinct as i64) < total)
}

/// A listing that reached [`SIGNAL_PAGE_CEILING`] without the server
/// signalling its end.
fn page_ceiling_reached() -> SignalReadFailure {
    SignalReadFailure::new(
        reasons::PAGING_NOT_HONOURED,
        "incomplete played history: the server never signalled the end of the listing",
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
    /// [`SIGNAL_PAGE_CEILING`], lowered only by tests.
    page_ceiling: usize,
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
            page_ceiling: SIGNAL_PAGE_CEILING,
        }
    }

    #[cfg(test)]
    fn with_plex_service_url(plex_service_url: Url) -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_service_url,
            page_ceiling: SIGNAL_PAGE_CEILING,
        }
    }

    #[cfg(test)]
    fn with_page_ceiling(mut self, page_ceiling: usize) -> Self {
        self.page_ceiling = page_ceiling;
        self
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
        let mut pages = 0_usize;
        let mut first_total: Option<i64> = None;

        loop {
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
            first_total = first_total.or(total);
            if past_reported_total(start_index, first_total) {
                return Err(paging_not_honoured());
            }
            if !another_page_remains(start_index, page.len(), total)? {
                break;
            }
            pages += 1;
            if pages >= self.page_ceiling {
                return Err(page_ceiling_reached());
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
    ) -> Result<BTreeMap<String, Option<String>>, FetchFailure> {
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
            self.for_each_plex_page(url, auth, true, |entries| {
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
                Ok(())
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
        Ok(self
            .fetch_plex_history_plays(connection, base, auth, Some(account_id), None)
            .await?
            .into_iter()
            .map(|(_, item)| item)
            .collect())
    }

    /// Server history read with the owner's token, folded per (account, item)
    /// into a count and the newest `viewedAt`, each item described by its
    /// library metadata. `account` filters to one account; `None` reads every
    /// account the server has. History holds organic plays only: an item
    /// marked watched, or scrobbled, without being played is not in it.
    ///
    /// `owner_id`, when given, is the owner's plex.tv id: history names the
    /// owner as the server's local account `1`, and their plays are folded
    /// under the plex.tv id the owner's linked user and watched set use.
    ///
    /// Read oldest first, so a play that lands mid-read joins the end of the
    /// listing rather than shifting every later page; see
    /// [`plex_play_fingerprint`] for how a repeated row is placed.
    ///
    /// Items are looked up page by page as they first appear, so only the
    /// distinct items are held, never the rows. A row that names no item or
    /// no type, or no account on an unfiltered read, cannot be credited and
    /// fails the read as `WATCH_SYNC_ITEMS_UNREADABLE`. On a read filtered to
    /// one account, a row naming no account or another one shows the filter
    /// did not hold, and fails as `WATCH_SYNC_FILTER_NOT_HONOURED`.
    async fn fetch_plex_history_plays(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        auth: SignalAuth<'_>,
        account: Option<&str>,
        owner_id: Option<&str>,
    ) -> ReadResult<Vec<(String, ProviderPlayedItem)>> {
        let mut url = plex_url(base, "status/sessions/history/all")?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(account_id) = account {
                query.append_pair("accountID", account_id);
            }
            query.append_pair("sort", "viewedAt:asc");
        }

        let mut plays: BTreeMap<(String, String), PlexPlays> = BTreeMap::new();
        let mut described: BTreeMap<String, ProviderPlayedItem> = BTreeMap::new();
        let mut looked_up: HashSet<String> = HashSet::new();
        // python-plexapi pages history by container headers and totalSize and
        // never reads `offset`, so a repeated play, not an `offset` echo,
        // places a history page.
        let mut seen_plays: HashSet<u64> = HashSet::new();
        let mut pager = PlexPager::new(url, false);
        while let Some(entries) = pager.next_page(self, auth).await? {
            // The owner's token can read every account's history, so a server
            // that ignored `accountID` would hand this account everyone's
            // plays, and a completion rule would then read episodes as
            // finished by someone who never saw them. A row that names no
            // account cannot show the filter held, so it fails the same way.
            if let Some(account_id) = account
                && entries.iter().any(|entry| {
                    entry
                        .get("accountID")
                        .and_then(scalar_string)
                        .is_none_or(|row_account| row_account != account_id)
                })
            {
                return Err(SignalReadFailure::new(
                    reasons::FILTER_NOT_HONOURED,
                    "incomplete played history: the account filter was not applied",
                ));
            }
            let page_len = entries.len();
            let entries: Vec<Value> = entries
                .into_iter()
                .filter(|entry| seen_plays.insert(plex_play_fingerprint(entry)))
                .collect();
            if page_len > 0 && entries.is_empty() {
                // Every row already seen: the page was answered from
                // elsewhere, not shifted by a play that came or went.
                return Err(paging_not_honoured());
            }
            let skipped = fold_plex_history_into(&entries, &mut plays, owner_id);
            if skipped > 0 {
                warn!(
                    connection_id = connection.id.as_str(),
                    skipped, "skipped unreadable Plex history entries"
                );
                return Err(unreadable_items());
            }

            let new_keys = entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.get("type").and_then(Value::as_str),
                        Some("movie" | "episode")
                    )
                })
                .filter_map(|entry| entry.get("ratingKey").and_then(scalar_string))
                .filter(|key| looked_up.insert(key.clone()))
                .collect::<Vec<_>>();
            for chunk in new_keys.chunks(LOOKUP_BATCH) {
                let mut unreadable = 0_usize;
                for entry in self.plex_metadata(base, auth, chunk).await? {
                    match normalize_plex_item(&entry) {
                        Some(item) => {
                            described.insert(item.provider_item_id.clone(), item);
                        }
                        None => unreadable += 1,
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
            }
        }
        if history_read_short_of_total(seen_plays.len(), pager.last_total) {
            return Err(paging_not_honoured());
        }

        let mut items = Vec::with_capacity(plays.len());
        let mut gone = 0_usize;
        for ((account_id, rating_key), play) in plays {
            let Some(item) = described.get(&rating_key) else {
                gone += 1;
                continue;
            };
            // The metadata was read with the owner's token, so its own view
            // count and last-viewed time are the owner's. Each account's come
            // from its history alone.
            let mut item = item.clone();
            item.played = true;
            item.play_count = play.count;
            item.last_played_at = play.last_viewed_at;
            items.push((account_id, item));
        }
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
    /// incomplete when the listing cannot be read whole. See [`PlexPager`].
    async fn for_each_plex_page(
        &self,
        url: Url,
        auth: SignalAuth<'_>,
        offset_required: bool,
        mut consume: impl FnMut(Vec<Value>) -> ReadResult<()>,
    ) -> ReadResult<Option<i64>> {
        let mut pager = PlexPager::new(url, offset_required);
        while let Some(entries) = pager.next_page(self, auth).await? {
            consume(entries)?;
        }
        Ok(pager.last_total)
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
        match self.plex_metadata_typed(base, auth, keys).await {
            Ok(entries) => Ok(entries),
            Err(MetadataFailure::Read(failure)) => Err(failure),
            Err(MetadataFailure::Fetch(failure)) => Err(failure.into_failure()),
        }
    }

    /// [`Self::plex_metadata`] with a refusal still typed, for the re-check of
    /// a shared user, where a refusal is that user's answer and not the
    /// connection's failure.
    async fn plex_metadata_typed(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        keys: &[String],
    ) -> Result<Vec<Value>, MetadataFailure> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        match self
            .request_json(plex_metadata_url(base, keys)?, auth, None)
            .await
        {
            Ok(body) => Ok(plex_metadata_entries(&body)?),
            Err(FetchFailure::Status(StatusCode::NOT_FOUND)) if keys.len() > 1 => {
                let mut entries = Vec::new();
                for key in keys {
                    let url = plex_metadata_url(base, std::slice::from_ref(key))?;
                    match self.request_json(url, auth, None).await {
                        Ok(body) => entries.extend(plex_metadata_entries(&body)?),
                        Err(FetchFailure::Status(StatusCode::NOT_FOUND)) => {}
                        Err(failure) => return Err(MetadataFailure::Fetch(failure)),
                    }
                }
                Ok(entries)
            }
            Err(FetchFailure::Status(StatusCode::NOT_FOUND)) => Ok(Vec::new()),
            Err(failure) => Err(MetadataFailure::Fetch(failure)),
        }
    }

    /// Each key's item as `auth`'s account sees it, read in batches of
    /// [`LOOKUP_BATCH`]. A key the account cannot see is absent. A refusal of
    /// a whole batch (404 or 403) is `refused` for every key in it when
    /// `refusal_is_unplayed`; otherwise it fails the read.
    async fn plex_item_states(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        keys: &[String],
        refusal_is_unplayed: bool,
    ) -> ReadResult<BTreeMap<String, ProviderPlayedItem>> {
        let mut states = BTreeMap::new();
        for chunk in keys.chunks(LOOKUP_BATCH) {
            let entries = match self.plex_metadata_typed(base, auth, chunk).await {
                Ok(entries) => entries,
                Err(MetadataFailure::Fetch(FetchFailure::Status(
                    StatusCode::NOT_FOUND | StatusCode::FORBIDDEN,
                ))) if refusal_is_unplayed => continue,
                Err(MetadataFailure::Read(failure)) => return Err(failure),
                Err(MetadataFailure::Fetch(failure)) => return Err(failure.into_failure()),
            };
            for entry in &entries {
                let item = normalize_plex_item(entry).ok_or_else(unreadable_items)?;
                if !chunk.contains(&item.provider_item_id) {
                    return Err(SignalReadFailure::new(
                        reasons::FILTER_NOT_HONOURED,
                        "item re-check: the server answered for an item not asked about",
                    ));
                }
                states.insert(item.provider_item_id.clone(), item);
            }
        }
        Ok(states)
    }

    /// Every play the server's history holds for one item, for any account,
    /// read with the owner's token for a pre-action re-check. Organic plays
    /// only, as for the sweep's history read.
    ///
    /// Read oldest first, with a repeated row skipped, and a page of nothing
    /// but repeats or a read of fewer distinct rows than the last reported
    /// total failing, exactly as [`Self::fetch_plex_history_plays`] reads. The owner's local account `1` is reported as `owner_id`.
    ///
    /// A row for another item means the item filter was not applied, and a
    /// row that names no account or no time cannot be credited to anyone;
    /// both fail the re-check rather than being passed over.
    async fn fetch_plex_item_history(
        &self,
        base: &Url,
        auth: SignalAuth<'_>,
        rating_key: &str,
        owner_id: &str,
    ) -> ReadResult<Vec<ProviderItemPlay>> {
        let mut url = plex_url(base, "status/sessions/history/all")?;
        url.query_pairs_mut()
            .append_pair("metadataItemID", rating_key)
            .append_pair("sort", "viewedAt:asc");
        let mut plays = Vec::new();
        let mut seen_plays: HashSet<u64> = HashSet::new();
        let last_total = self
            .for_each_plex_page(url, auth, false, |entries| {
                let page_len = entries.len();
                let entries: Vec<Value> = entries
                    .into_iter()
                    .filter(|entry| seen_plays.insert(plex_play_fingerprint(entry)))
                    .collect();
                if page_len > 0 && entries.is_empty() {
                    return Err(paging_not_honoured());
                }
                for entry in &entries {
                    if entry
                        .get("ratingKey")
                        .and_then(scalar_string)
                        .is_some_and(|row_key| row_key != rating_key)
                    {
                        return Err(SignalReadFailure::new(
                            reasons::FILTER_NOT_HONOURED,
                            "item history: the item filter was not applied",
                        ));
                    }
                    let account = entry.get("accountID").and_then(scalar_string);
                    let viewed_at = entry.get("viewedAt").and_then(epoch_timestamp);
                    let (Some(account), Some(viewed_at)) = (account, viewed_at) else {
                        return Err(unreadable_items());
                    };
                    plays.push(ProviderItemPlay {
                        external_user_id: plex_history_account(account, Some(owner_id)),
                        played: true,
                        last_played_at: Some(viewed_at),
                    });
                }
                Ok(())
            })
            .await?;
        if history_read_short_of_total(seen_plays.len(), last_total) {
            return Err(paging_not_honoured());
        }
        Ok(plays)
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

impl UserItemsReader {
    /// Every user the admin key can list (`GET /Users`), as an array or an
    /// `Items` wrapper. A user the list cannot name fails the read: their
    /// plays would otherwise be missing from an answer that looks complete.
    async fn list_user_ids(&self) -> ReadResult<Vec<String>> {
        let (auth, _) = self.auth_and_fields();
        let url = self.base.join("Users").map_err(|_| invalid_base_url())?;
        let body = self.source.fetch_json(url, auth, None).await?;
        body.as_array()
            .or_else(|| field(&body, "Items").and_then(Value::as_array))
            .ok_or_else(unreadable_listing)?
            .iter()
            .map(|user| string_field(user, "Id").ok_or_else(unreadable_listing))
            .collect()
    }

    /// The admin key's credential header and the item fields this provider
    /// is asked for.
    fn auth_and_fields(&self) -> (SignalAuth<'_>, &'static str) {
        match self.connection.provider {
            MediaServerProvider::Emby => (
                SignalAuth::Emby {
                    token: &self.token,
                    connection_id: &self.connection.id,
                },
                EMBY_ITEM_FIELDS,
            ),
            _ => (SignalAuth::Jellyfin(&self.token), JELLYFIN_ITEM_FIELDS),
        }
    }
}

#[async_trait]
impl MediaServerSignalReader for UserItemsReader {
    async fn fetch_played_items(
        &self,
        external_user_id: &str,
    ) -> Result<Vec<ProviderPlayedItem>, SignalReadFailure> {
        let external_user_id = participant_id(external_user_id)?;
        let (auth, fields) = self.auth_and_fields();
        self.source
            .fetch_user_items(&self.connection, &self.base, auth, fields, external_user_id)
            .await
    }

    /// Every user the admin key can list (`GET /Users`), each read exactly as
    /// a participant is. Both servers keep played state per user and no
    /// server-wide history, so this is the whole audience. A user the list
    /// cannot name fails the read: their plays would otherwise be missing
    /// from an answer that looks complete.
    async fn fetch_audience_plays(&self) -> Result<Vec<ProviderAudiencePlay>, SignalReadFailure> {
        let (auth, fields) = self.auth_and_fields();
        let mut plays = Vec::new();
        for user_id in self.list_user_ids().await? {
            let items = self
                .source
                .fetch_user_items(&self.connection, &self.base, auth, fields, &user_id)
                .await?;
            plays.extend(items.into_iter().map(|item| ProviderAudiencePlay {
                external_account_id: user_id.clone(),
                item,
            }));
        }
        Ok(plays)
    }

    /// Each item's user data for every user the admin key can list
    /// (`GET /Users`) and every participant, read per user in batches of
    /// [`LOOKUP_BATCH`] ids through `/Users/{userId}/Items?Ids=`. Neither
    /// server keeps a server-wide play history, so every listed user's own
    /// state is what stands in for one: each user who has played an item is
    /// one of its history plays, and a play no stored fact credits holds the
    /// action exactly as a Plex history row does.
    ///
    /// A user the server refuses (404 or 403) or who cannot see an item has
    /// no plays of it. A refusal of every read is the admin key's, not the
    /// users', and fails; so does an item no user can see, because a stored
    /// id the server no longer knows cannot be shown unplayed.
    async fn recheck_items(
        &self,
        provider_item_ids: &[String],
        participants: &[String],
    ) -> Result<BTreeMap<String, ProviderItemRecheck>, SignalReadFailure> {
        let (auth, fields) = self.auth_and_fields();
        let ids = recheck_item_ids(provider_item_ids)?;
        let users = self.list_user_ids().await?;
        let mut readers: Vec<String> = users.clone();
        for participant in participants {
            let participant = participant_id(participant)?.to_string();
            if !readers.contains(&participant) {
                readers.push(participant);
            }
        }

        let source = &self.source;
        let base = &self.base;
        let jobs: Vec<(String, Vec<String>)> = readers
            .iter()
            .flat_map(|user| {
                ids.chunks(LOOKUP_BATCH)
                    .map(move |chunk| (user.clone(), chunk.to_vec()))
            })
            .collect();
        let reads = jobs.into_iter().map(|(user, chunk)| async move {
            let url = items_by_id_url(base, &user, &chunk, fields)?;
            let entries = match source.request_json(url, auth, None).await {
                Ok(body) => field(&body, "Items")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(unreadable_listing)?,
                Err(FetchFailure::Status(StatusCode::NOT_FOUND | StatusCode::FORBIDDEN)) => {
                    return Ok::<_, SignalReadFailure>((user, None));
                }
                Err(failure) => return Err(failure.into_failure()),
            };
            let mut states = Vec::with_capacity(entries.len());
            for entry in &entries {
                let id = string_field(entry, "Id").ok_or_else(unreadable_items)?;
                if !chunk.contains(&id) {
                    return Err(SignalReadFailure::new(
                        reasons::FILTER_NOT_HONOURED,
                        "item re-check: the server answered for an item not asked about",
                    ));
                }
                states.push((id, user_item_state(entry, &user)?));
            }
            Ok((user, Some(states)))
        });
        let answers: Vec<(String, Option<Vec<(String, ProviderItemPlay)>>)> =
            futures_util::stream::iter(reads)
                .buffer_unordered(RECHECK_CONCURRENCY)
                .try_collect()
                .await?;
        if !answers.is_empty() && answers.iter().all(|(_, states)| states.is_none()) {
            return Err(SignalReadFailure::new(
                reasons::ACCESS_DENIED,
                "item re-check: every user read was refused",
            ));
        }

        let mut states: BTreeMap<(String, String), ProviderItemPlay> = BTreeMap::new();
        for (user, answer) in answers {
            for (id, state) in answer.into_iter().flatten() {
                states.insert((user.clone(), id), state);
            }
        }
        let mut rechecks = BTreeMap::new();
        for id in ids {
            if !states.keys().any(|(_, seen)| *seen == id) {
                return Err(SignalReadFailure::new(
                    reasons::RESPONSE_UNREADABLE,
                    "the item is not on the server any more",
                ));
            }
            let state_of = |user: &str| {
                states
                    .get(&(user.to_string(), id.clone()))
                    .cloned()
                    .unwrap_or_else(|| ProviderItemPlay {
                        external_user_id: user.to_string(),
                        played: false,
                        last_played_at: None,
                    })
            };
            let recheck = ProviderItemRecheck {
                participants: participants
                    .iter()
                    .map(|participant| state_of(participant.trim()))
                    .collect(),
                history: users
                    .iter()
                    .map(|user| state_of(user))
                    .filter(|state| state.played)
                    .collect(),
            };
            rechecks.insert(id, recheck);
        }
        Ok(rechecks)
    }
}

/// One user's state from a single-item answer. The answer must be the item
/// asked for and carry its user-data block: without it there is nothing to
/// show the item unplayed.
fn user_item_state(body: &Value, external_user_id: &str) -> ReadResult<ProviderItemPlay> {
    let user_data = field(body, "UserData").filter(|data| data.is_object());
    let Some(user_data) = user_data else {
        return Err(SignalReadFailure::new(
            reasons::RESPONSE_UNREADABLE,
            "the item answer carried no user data",
        ));
    };
    let played = field(user_data, "Played")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            SignalReadFailure::new(
                reasons::RESPONSE_UNREADABLE,
                "the item answer did not say whether it was played",
            )
        })?;
    Ok(ProviderItemPlay {
        external_user_id: external_user_id.to_string(),
        played,
        last_played_at: field(user_data, "LastPlayedDate")
            .and_then(Value::as_str)
            .and_then(parse_timestamp),
    })
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
    /// connection and only if a non-owner participant needs one, or the
    /// failure that kept them from being listed.
    shared_tokens: OnceCell<ReadResult<BTreeMap<String, Option<String>>>>,
}

impl PlexReader {
    /// The server access token of a user the server is shared with, if plex.tv
    /// lists one for them.
    ///
    /// A list that could not be read fails the read instead of answering "no
    /// token". History is not the same source as the library: it holds only
    /// organic plays, so a shared user read from it in place of their own
    /// library view would lose every item they marked watched, and the sweep
    /// would still record a clean, smaller watch history.
    async fn shared_token(&self, account_id: &str) -> ReadResult<Option<String>> {
        let tokens = self
            .shared_tokens
            .get_or_init(|| async {
                let machine_id = self
                    .connection
                    .machine_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        SignalReadFailure::new(
                            reasons::PLEX_SERVER_NOT_SELECTED,
                            "no Plex server selected",
                        )
                    })?;
                self.source
                    .plex_shared_server_tokens(machine_id, &self.owner_token)
                    .await
                    .map_err(|failure| {
                        let failure = failure.into_failure();
                        warn!(
                            connection_id = self.connection.id.as_str(),
                            reason = failure.reason,
                            error = %failure,
                            "could not list Plex shared-server tokens; users other than the owner are not read"
                        );
                        failure
                    })
            })
            .await;
        match tokens {
            Ok(tokens) => match tokens.get(account_id) {
                Some(Some(token)) => Ok(Some(token.clone())),
                Some(None) => Err(SignalReadFailure::new(
                    reasons::RESPONSE_UNREADABLE,
                    "plex.tv lists this user as shared with no usable server token",
                )),
                None => Ok(None),
            },
            Err(failure) => Err(failure.clone()),
        }
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
        let shared_token = self.shared_token(external_user_id).await?;
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

    /// Every account's plays: the owner's own library listing, keyed by the
    /// owner's plex.tv id, and the server's history across every account,
    /// keyed by the account id history names. History names the owner as the
    /// server's local account `1`; those plays are folded under the owner's
    /// plex.tv id, so the owner's plays from both sources are one account's.
    /// Both are read with the owner's token.
    ///
    /// History keeps organic plays only: no marks and no scrobbles. An item a
    /// shared or managed user marked watched without playing is therefore not
    /// in the audience; the owner's marks are, from the listing.
    async fn fetch_audience_plays(&self) -> Result<Vec<ProviderAudiencePlay>, SignalReadFailure> {
        let owner_auth = SignalAuth::Plex(&self.owner_token);
        let source = &self.source;
        let owner_id = self.owner_ids.first().cloned().ok_or_else(|| {
            SignalReadFailure::new(reasons::RESPONSE_UNREADABLE, "unreadable Plex account")
        })?;

        let mut merged: BTreeMap<(String, String), ProviderPlayedItem> = BTreeMap::new();
        let listing = source
            .fetch_plex_watched_set(&self.connection, &self.base, owner_auth)
            .await?
            .into_iter()
            .map(|item| (owner_id.clone(), item));
        let history = source
            .fetch_plex_history_plays(
                &self.connection,
                &self.base,
                owner_auth,
                None,
                Some(&owner_id),
            )
            .await?;
        for (account, item) in listing.chain(history) {
            match merged.entry((account, item.provider_item_id.clone())) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(item);
                }
                // One account's item from both sources: the larger count and
                // the newer time, so neither source hides the other.
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    let kept = slot.get_mut();
                    kept.play_count = kept.play_count.max(item.play_count);
                    kept.last_played_at = kept.last_played_at.max(item.last_played_at);
                }
            }
        }

        let (accounts, mut items): (Vec<String>, Vec<ProviderPlayedItem>) = merged
            .into_iter()
            .map(|((account, _), item)| (account, item))
            .unzip();
        source
            .attach_plex_series_ids(&self.base, owner_auth, &mut items)
            .await;
        Ok(accounts
            .into_iter()
            .zip(items)
            .map(|(external_account_id, item)| ProviderAudiencePlay {
                external_account_id,
                item,
            })
            .collect())
    }

    /// The server's history for each item, read with the owner's token so it
    /// names every account's plays (organic plays only; history holds no
    /// marks or scrobbles), and each participant's own state.
    ///
    /// History takes one request per item, kept to [`RECHECK_CONCURRENCY`]
    /// in flight. Item state is read in batches through
    /// `/library/metadata/{keys}`: with the owner's token for the owner, and
    /// to prove every item is still on the server; with a shared user's own
    /// token for them, where a refusal or an item hidden from them is no play;
    /// and, for a user read from history, from that same history.
    async fn recheck_items(
        &self,
        provider_item_ids: &[String],
        participants: &[String],
    ) -> Result<BTreeMap<String, ProviderItemRecheck>, SignalReadFailure> {
        let keys = recheck_item_ids(provider_item_ids)?;
        let source = &self.source;
        let base = &self.base;
        let owner_auth = SignalAuth::Plex(&self.owner_token);
        let owner_id = self.owner_ids.first().cloned().ok_or_else(|| {
            SignalReadFailure::new(reasons::RESPONSE_UNREADABLE, "unreadable Plex account")
        })?;

        let owner_states = source
            .plex_item_states(base, owner_auth, &keys, false)
            .await?;
        if keys.iter().any(|key| !owner_states.contains_key(key)) {
            return Err(SignalReadFailure::new(
                reasons::RESPONSE_UNREADABLE,
                "the item is not on the server any more",
            ));
        }
        let owner_id_ref = owner_id.as_str();
        let histories: Vec<(String, Vec<ProviderItemPlay>)> =
            futures_util::stream::iter(keys.clone().into_iter().map(|key| async move {
                let history = source
                    .fetch_plex_item_history(base, owner_auth, &key, owner_id_ref)
                    .await?;
                Ok::<_, SignalReadFailure>((key, history))
            }))
            .buffer_unordered(RECHECK_CONCURRENCY)
            .try_collect()
            .await?;
        let mut rechecks: BTreeMap<String, ProviderItemRecheck> = histories
            .into_iter()
            .map(|(key, history)| {
                (
                    key,
                    ProviderItemRecheck {
                        participants: Vec::with_capacity(participants.len()),
                        history,
                    },
                )
            })
            .collect();

        for participant in participants {
            let external_user_id = participant_id(participant)?;
            let unplayed = || ProviderItemPlay {
                external_user_id: external_user_id.to_string(),
                played: false,
                last_played_at: None,
            };
            let as_state = |item: &ProviderPlayedItem| ProviderItemPlay {
                external_user_id: external_user_id.to_string(),
                played: item.played,
                last_played_at: item.last_played_at,
            };
            if self.owner_ids.iter().any(|id| id == external_user_id) {
                for (key, recheck) in &mut rechecks {
                    recheck
                        .participants
                        .push(owner_states.get(key).map_or_else(unplayed, as_state));
                }
                continue;
            }
            if !external_user_id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(SignalReadFailure::new(
                    reasons::PLEX_ACCOUNT_ID_INVALID,
                    "Plex account id is not a server account id",
                ));
            }
            match self.shared_token(external_user_id).await? {
                Some(token) => {
                    let states = source
                        .plex_item_states(base, SignalAuth::Plex(&token), &keys, true)
                        .await?;
                    for (key, recheck) in &mut rechecks {
                        recheck
                            .participants
                            .push(states.get(key).map_or_else(unplayed, as_state));
                    }
                }
                None => {
                    for recheck in rechecks.values_mut() {
                        let last_played_at = recheck
                            .history
                            .iter()
                            .filter(|play| play.external_user_id == external_user_id)
                            .filter_map(|play| play.last_played_at)
                            .max();
                        recheck.participants.push(ProviderItemPlay {
                            external_user_id: external_user_id.to_string(),
                            played: last_played_at.is_some(),
                            last_played_at,
                        });
                    }
                }
            }
        }
        Ok(rechecks)
    }
}

/// `userID` → `accessToken` from plex.tv's shared-server list.
///
/// A user the list names without a usable token maps to `None`: reading them
/// from history instead would drop every play they marked by hand, so their
/// read fails while everyone else's token still counts.
///
/// `None` for the whole list when the body cannot be trusted as that list:
/// unreadable XML, or a root other than `MediaContainer` (a well-formed error
/// or login page is not "no shared users").
fn shared_server_tokens(xml: &str) -> Option<BTreeMap<String, Option<String>>> {
    let mut reader = Reader::from_str(xml);
    let mut tokens = BTreeMap::new();
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element) | Event::Empty(element)) => {
                if !saw_root {
                    if element.name().as_ref() != "MediaContainer" {
                        return None;
                    }
                    saw_root = true;
                }
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
                let Some(user_id) = user_id.filter(|id| !id.is_empty()) else {
                    continue;
                };
                let access_token = access_token.filter(|token| !token.is_empty());
                // A second entry for the same user with a token is still that
                // user's token; one without never erases it.
                let entry = tokens.entry(user_id).or_insert(None);
                if entry.is_none() {
                    *entry = access_token;
                }
            }
            Ok(Event::Eof) => return saw_root.then_some(tokens),
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

/// Fold history entries into plays per (account, rating key). Returns
/// `(plays, skipped)`.
///
/// An entry with no account, no rating key or no type is unreadable. An entry
/// of a type this adapter does not record (music, photos) is not a movie or
/// episode play at all, so it is passed over rather than counted.
#[cfg(test)]
fn fold_plex_history(
    entries: &[Value],
    owner_id: Option<&str>,
) -> (BTreeMap<(String, String), PlexPlays>, usize) {
    let mut plays = BTreeMap::new();
    let skipped = fold_plex_history_into(entries, &mut plays, owner_id);
    (plays, skipped)
}

/// The account a history row's play belongs to. History names the server's
/// owner as the local account `1`; with the owner's plex.tv id known, that
/// play is the owner's, under the id their linked user and watched set use.
/// Every other account id is kept as history gives it.
fn plex_history_account(account: String, owner_id: Option<&str>) -> String {
    match owner_id {
        Some(owner_id) if account == PLEX_LOCAL_OWNER_ACCOUNT => owner_id.to_string(),
        _ => account,
    }
}

/// Fold one page of history entries into `plays`, the owner's local account
/// read as `owner_id` (see [`plex_history_account`]). Returns how many were
/// unreadable.
fn fold_plex_history_into(
    entries: &[Value],
    plays: &mut BTreeMap<(String, String), PlexPlays>,
    owner_id: Option<&str>,
) -> usize {
    let mut skipped = 0_usize;
    for entry in entries {
        let (Some(account_id), Some(rating_key)) = (
            entry.get("accountID").and_then(scalar_string),
            entry.get("ratingKey").and_then(scalar_string),
        ) else {
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
        let account_id = plex_history_account(account_id, owner_id);
        let play = plays.entry((account_id, rating_key)).or_default();
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

/// The distinct, trimmed ids a re-check was asked about. A blank id is a
/// stored item with nothing to read it by.
fn recheck_item_ids(provider_item_ids: &[String]) -> ReadResult<Vec<String>> {
    let mut ids: Vec<String> = Vec::with_capacity(provider_item_ids.len());
    for id in provider_item_ids {
        let id = id.trim();
        if id.is_empty() {
            return Err(SignalReadFailure::new(
                reasons::RESPONSE_UNREADABLE,
                "the stored item has no id",
            ));
        }
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

/// One user's view of a batch of items by id, with their user data.
/// `Recursive=true` for the same reason as [`series_lookup_url`].
fn items_by_id_url(
    base: &Url,
    external_user_id: &str,
    ids: &[String],
    fields: &str,
) -> ReadResult<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| invalid_base_url())?
        .pop_if_empty()
        .extend(["Users", external_user_id, "Items"]);
    url.query_pairs_mut()
        .append_pair("Recursive", "true")
        .append_pair("Ids", &ids.join(","))
        .append_pair("Fields", fields)
        .append_pair("EnableUserData", "true")
        .append_pair("Limit", &ids.len().to_string());
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

/// Why a metadata read failed: a request the caller may still classify, or an
/// answer that was already judged unreadable.
#[derive(Debug)]
enum MetadataFailure {
    Fetch(FetchFailure),
    Read(SignalReadFailure),
}

impl From<SignalReadFailure> for MetadataFailure {
    fn from(failure: SignalReadFailure) -> Self {
        Self::Read(failure)
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
    /// for; `distinct_keys` folds Plex history rows onto that many items;
    /// `growing_total` reports a total two pages past every page it serves,
    /// so the listing never reaches it.
    struct GeneratedPages {
        plex: bool,
        total: usize,
        report_total: bool,
        page_limit: usize,
        distinct_keys: Option<usize>,
        growing_total: bool,
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
                            "accountID": 2002,
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
            let reported_total = if self.growing_total {
                start.saturating_add(2 * self.page_limit)
            } else {
                self.total
            };
            let body = if self.plex {
                let mut container =
                    json!({ "size": rows.len(), "offset": start, "Metadata": rows });
                if self.report_total {
                    container["totalSize"] = json!(reported_total);
                }
                json!({ "MediaContainer": container })
            } else {
                let mut body = json!({ "Items": rows });
                if self.report_total {
                    body["TotalRecordCount"] = json!(reported_total);
                }
                body
            };
            ResponseTemplate::new(200).set_body_json(body)
        }
    }

    /// More played items than a sweep once stopped at, for the tests that
    /// show there is no ceiling any more.
    const LARGE_HISTORY_ITEMS: usize = 25_000;

    /// Plex metadata for whatever rating keys the request path names, each a
    /// movie with a TMDB id equal to its key.
    struct GeneratedMetadata;

    impl wiremock::Respond for GeneratedMetadata {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let keys = request
                .url
                .path()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .split(',')
                .map(|key| {
                    json!({
                        "ratingKey": key,
                        "type": "movie",
                        "Guid": [{ "id": format!("tmdb://{key}") }]
                    })
                })
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": keys.len(), "Metadata": keys }
            }))
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

    /// Every partial Emby read — an unreadable item, a total never reached, a page the
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

        // A server whose total keeps climbing ahead of every page it serves.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(GeneratedPages {
                plex: false,
                total: usize::MAX,
                report_total: true,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
                growing_total: true,
            })
            .mount(&server)
            .await;
        let error = HttpMediaServerSignalSource::new()
            .read_played(&emby_connection(server.uri()), "emby-user-1")
            .await
            .expect_err("a total that is never reached is not paging as asked");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);

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
                growing_total: false,
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

    /// plex.tv lists no server token for anyone: every non-owner is read from
    /// history.
    async fn mount_no_shared_server_tokens(server: &wiremock::MockServer) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"<MediaContainer size="0" />"#),
            )
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
    fn folds_plex_history_into_plays_per_account_and_item() {
        let (plays, skipped) = fold_plex_history(
            &[
                json!({ "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 }),
                json!({ "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_500_000 }),
                // The same item played by another account is that account's play.
                json!({ "ratingKey": "101", "type": "movie", "accountID": "3003", "viewedAt": 1_700_200_000 }),
                json!({ "ratingKey": 201, "type": "episode", "accountID": 2002, "viewedAt": "1700100000" }),
                // Music is not a movie or episode play: passed over, not unreadable.
                json!({ "ratingKey": "301", "type": "track", "accountID": 2002, "viewedAt": 1_700_000_000 }),
                // No rating key, no type, and no account: all unreadable.
                json!({ "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 }),
                json!({ "ratingKey": "102", "accountID": 2002, "viewedAt": 1_700_000_000 }),
                json!({ "ratingKey": "103", "type": "movie", "viewedAt": 1_700_000_000 }),
            ],
            None,
        );
        let key = |account: &str, item: &str| (account.to_string(), item.to_string());
        assert_eq!(skipped, 3);
        assert_eq!(plays.len(), 3);
        assert_eq!(
            plays.get(&key("2002", "101")),
            Some(&PlexPlays {
                count: 2,
                last_viewed_at: DateTime::from_timestamp(1_700_500_000, 0),
            })
        );
        assert_eq!(
            plays.get(&key("3003", "101")).map(|play| play.count),
            Some(1)
        );
        assert_eq!(
            plays.get(&key("2002", "201")).map(|play| play.count),
            Some(1)
        );
    }

    /// History names the server owner as the local account `1`. With the
    /// owner's plex.tv id known, those rows are the owner's plays under that
    /// id; every other account keeps its own id, and without an owner id
    /// nothing is renamed.
    #[test]
    fn plex_history_folds_the_local_owner_account_into_the_owners_plex_id() {
        let rows = [
            json!({ "ratingKey": "101", "type": "movie", "accountID": 1, "viewedAt": 1_700_000_000 }),
            json!({ "ratingKey": "101", "type": "movie", "accountID": "1001", "viewedAt": 1_700_100_000 }),
            json!({ "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_200_000 }),
        ];
        let key = |account: &str| (account.to_string(), "101".to_string());

        let (plays, _) = fold_plex_history(&rows, Some(PLEX_OWNER_ID));
        assert_eq!(plays.len(), 2);
        assert_eq!(
            plays.get(&key(PLEX_OWNER_ID)).map(|play| play.count),
            Some(2)
        );
        assert_eq!(
            plays.get(&key(PLEX_SHARED_ID)).map(|play| play.count),
            Some(1)
        );
        assert!(!plays.contains_key(&key("1")));

        let (plays, _) = fold_plex_history(&rows, None);
        assert_eq!(plays.get(&key("1")).map(|play| play.count), Some(1));
        assert_eq!(
            plays.get(&key(PLEX_OWNER_ID)).map(|play| play.count),
            Some(1)
        );
        assert_eq!(
            plex_history_account("2002".into(), Some(PLEX_OWNER_ID)),
            "2002"
        );
        assert_eq!(
            plex_history_account("1".into(), Some(PLEX_OWNER_ID)),
            PLEX_OWNER_ID
        );
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
            mount_no_shared_server_tokens(&server).await;
            Mock::given(method("GET"))
                .and(path("/status/sessions/history/all"))
                .respond_with(body)
                .mount(&server)
                .await;
            plex_source(&server)
                .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
                .await
                .expect_err("a partial history is not a complete one")
        }

        // An entry with no rating key.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": 1, "Metadata": [
                { "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 }
            ] }
        })))
        .await;
        assert_eq!(error.reason, reasons::ITEMS_UNREADABLE);

        // A server whose total keeps climbing ahead of every page it serves.
        let error = history_error(GeneratedPages {
            plex: true,
            total: usize::MAX,
            report_total: true,
            page_limit: SIGNAL_PAGE_SIZE,
            distinct_keys: None,
            growing_total: true,
        })
        .await;
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);

        // A server that stops short of the total it reported.
        let error = history_error(ResponseTemplate::new(200).set_body_json(json!({
            "MediaContainer": { "size": 0, "offset": 0, "totalSize": 3, "Metadata": [] }
        })))
        .await;
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);

        let full_page = (0..SIGNAL_PAGE_SIZE)
            .map(|index| json!({ "ratingKey": index.to_string(), "type": "movie", "accountID": 2002, "viewedAt": 1_700_000_000 }))
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
                growing_total: false,
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

    /// There is no ceiling on one person's history: a participant with more
    /// played items than a sweep used to stop at is read whole.
    #[tokio::test]
    async fn a_participant_with_a_very_large_history_is_read_whole() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(GeneratedPages {
                plex: false,
                total: LARGE_HISTORY_ITEMS,
                report_total: true,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
                growing_total: false,
            })
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .read_played(&connection, "user-123")
            .await
            .expect("a large history is a complete read");
        assert_eq!(items.len(), LARGE_HISTORY_ITEMS);
    }

    /// Plex history has a row per play. A heavy rewatcher with more rows, over
    /// more items, than a sweep used to stop at is read whole, each item
    /// looked up once as it first appears.
    #[tokio::test]
    async fn plex_history_of_a_very_heavy_rewatcher_is_read_whole() {
        use wiremock::matchers::{method, path, path_regex};
        use wiremock::{Mock, MockServer};

        const PLAYS_PER_ITEM: usize = 9;
        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_no_shared_server_tokens(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(GeneratedPages {
                plex: true,
                total: LARGE_HISTORY_ITEMS * PLAYS_PER_ITEM,
                report_total: true,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: Some(LARGE_HISTORY_ITEMS),
                growing_total: false,
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/library/metadata/[0-9,]+$"))
            .respond_with(GeneratedMetadata)
            .expect((LARGE_HISTORY_ITEMS / LOOKUP_BATCH) as u64)
            .mount(&server)
            .await;

        let items = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect("a very long history is a complete read");
        assert_eq!(items.len(), LARGE_HISTORY_ITEMS);
        assert!(
            items
                .iter()
                .all(|item| item.play_count == PLAYS_PER_ITEM as i64)
        );
    }

    /// Open the connection and read every account's plays, as a sweep does.
    async fn read_audience(
        source: &HttpMediaServerSignalSource,
        connection: &MediaServerConnection,
    ) -> ReadResult<Vec<ProviderAudiencePlay>> {
        source.open(connection).await?.fetch_audience_plays().await
    }

    /// A Plex server with one movie section whose watched listing holds the
    /// owner's one movie, and whose history answers `history`.
    async fn mount_plex_audience(server: &wiremock::MockServer, history: Value) {
        use wiremock::matchers::{method, path, query_param, query_param_is_missing};
        use wiremock::{Mock, ResponseTemplate};
        mount_plex_owner(server).await;
        mount_plex_show_metadata(server).await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "Directory": [
                    { "key": "1", "type": "movie", "title": "Movies" }
                ] }
            })))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "offset": 0, "totalSize": 1,
                    "Metadata": [plex_movie(2, 1_756_588_445)] }
            })))
            .mount(server)
            .await;
        // Every account's history: no account filter is sent.
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param_is_missing("accountID"))
            .and(query_param("sort", "viewedAt:asc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(history))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101,201"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 2, "Metadata": [plex_movie(9, 1), plex_episode()] }
            })))
            .mount(server)
            .await;
    }

    /// The audience is the owner's own listing plus history across every
    /// account, linked or not, each account's plays counted from its own
    /// rows rather than the owner's metadata.
    #[tokio::test]
    async fn plex_audience_reads_the_owners_listing_and_every_accounts_history() {
        let server = wiremock::MockServer::start().await;
        mount_plex_audience(
            &server,
            json!({ "MediaContainer": { "size": 4, "offset": 0, "totalSize": 4, "Metadata": [
                { "ratingKey": "101", "type": "movie", "accountID": 1, "viewedAt": 1_756_000_100 },
                { "ratingKey": "201", "type": "episode", "accountID": 2002, "viewedAt": 1_756_000_200 },
                { "ratingKey": "101", "type": "movie", "accountID": 3003, "viewedAt": 1_756_000_300 },
                { "ratingKey": "101", "type": "movie", "accountID": 3003, "viewedAt": 1_756_000_400 }
            ] } }),
        )
        .await;

        let mut plays = read_audience(&plex_source(&server), &plex_connection(server.uri()))
            .await
            .expect("the audience reads");
        plays.sort_by(|a, b| {
            (&a.external_account_id, &a.item.provider_item_id)
                .cmp(&(&b.external_account_id, &b.item.provider_item_id))
        });
        let summary = plays
            .iter()
            .map(|play| {
                (
                    play.external_account_id.as_str(),
                    play.item.provider_item_id.as_str(),
                    play.item.play_count,
                    play.item.last_played_at.map(|at| at.timestamp()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![
                // The owner's history row (local account `1`) merges into
                // the owner's listing.
                (PLEX_OWNER_ID, "101", 2, Some(1_756_588_445)),
                (PLEX_SHARED_ID, "201", 1, Some(1_756_000_200)),
                ("3003", "101", 2, Some(1_756_000_400)),
            ]
        );
        assert!(plays.iter().all(|play| play.item.played));
        let episode = plays
            .iter()
            .find(|play| play.item.kind == MediaServerSignalKind::Episode)
            .expect("the episode");
        assert_eq!(
            episode
                .item
                .series_external_ids
                .get("tvdb")
                .map(String::as_str),
            Some("999")
        );
    }

    /// A history row that names no account, or no item (as rows for removed
    /// media can), cannot be credited, so the audience is not read rather
    /// than read short.
    #[tokio::test]
    async fn plex_audience_with_an_uncreditable_history_row_is_unreadable() {
        for row in [
            json!({ "ratingKey": "101", "type": "movie", "viewedAt": 1_756_000_100 }),
            json!({ "type": "movie", "accountID": 3003, "viewedAt": 1_756_000_100 }),
        ] {
            let server = wiremock::MockServer::start().await;
            mount_plex_audience(
                &server,
                json!({ "MediaContainer": { "size": 1, "offset": 0, "totalSize": 1, "Metadata": [row] } }),
            )
            .await;
            let error = read_audience(&plex_source(&server), &plex_connection(server.uri()))
                .await
                .expect_err("an uncreditable row fails the read");
            assert_eq!(error.reason, reasons::ITEMS_UNREADABLE);
        }
    }

    /// Jellyfin's audience is every user the admin key lists, each read as a
    /// participant is, including users no one has linked.
    #[tokio::test]
    async fn jellyfin_audience_reads_every_listed_user() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "Id": "jf-user-a", "Name": "Example Viewer A" },
                { "Id": "jf-user-b", "Name": "Example Viewer B" }
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/Users/jf-user-a/Items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Items": [movie_item()], "TotalRecordCount": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/Users/jf-user-b/Items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Items": [], "TotalRecordCount": 0 })),
            )
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let plays = read_audience(&HttpMediaServerSignalSource::new(), &connection)
            .await
            .expect("the audience reads");
        assert_eq!(plays.len(), 1);
        assert_eq!(plays[0].external_account_id, "jf-user-a");
        assert_eq!(plays[0].item.provider_item_id, "jf-movie-1");
        assert_eq!(plays[0].item.play_count, 3);
    }

    /// Emby may wrap the user list in `Items`; the admin key travels in
    /// Emby's own header. A listed user with no id fails the read rather
    /// than going missing from it.
    #[tokio::test]
    async fn emby_audience_reads_a_wrapped_user_list_and_refuses_a_nameless_user() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users"))
            .and(header("X-Emby-Token", "api-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Items": [{ "Id": "emby-user-1" }] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/Users/emby-user-1/Items"))
            .and(header("X-Emby-Token", "api-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Items": [emby_movie_item()], "TotalRecordCount": 1 })),
            )
            .mount(&server)
            .await;
        let plays = read_audience(
            &HttpMediaServerSignalSource::new(),
            &emby_connection(server.uri()),
        )
        .await
        .expect("the audience reads");
        assert_eq!(plays.len(), 1);
        assert_eq!(plays[0].external_account_id, "emby-user-1");
        assert_eq!(plays[0].item.provider_item_id, "emby-movie-1");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{ "Name": "Example Viewer" }])),
            )
            .mount(&server)
            .await;
        let error = read_audience(
            &HttpMediaServerSignalSource::new(),
            &emby_connection(server.uri()),
        )
        .await
        .expect_err("a user the list cannot name fails the read");
        assert_eq!(error.reason, reasons::RESPONSE_UNREADABLE);
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

    /// When plex.tv cannot list the shared-server tokens, a shared user is not
    /// quietly read from history instead: history holds only organic plays,
    /// so an item they marked watched would vanish from a sweep that still
    /// reads as clean.
    #[tokio::test]
    async fn an_unlistable_shared_server_token_fails_the_read_rather_than_falling_back() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0, "offset": 0, "totalSize": 0 }
            })))
            .expect(0)
            .mount(&server)
            .await;

        let reader = plex_source(&server)
            .open(&plex_connection(server.uri()))
            .await
            .expect("opens");
        for account in [PLEX_SHARED_ID, "3003"] {
            let failure = reader
                .fetch_played_items(account)
                .await
                .expect_err("no token list, no read");
            assert_eq!(failure.reason, reasons::REQUEST_REJECTED);
        }
    }

    /// Without a selected server there is no shared-server list to ask, so
    /// no one but the owner can be read, and nobody falls back to history.
    #[tokio::test]
    async fn a_connection_without_a_selected_server_reads_no_one_but_the_owner() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<MediaContainer />"))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0, "offset": 0, "totalSize": 0 }
            })))
            .expect(0)
            .mount(&server)
            .await;

        let mut connection = plex_connection(server.uri());
        connection.machine_id = None;
        let reader = plex_source(&server).open(&connection).await.expect("opens");
        let failure = reader
            .fetch_played_items(PLEX_SHARED_ID)
            .await
            .expect_err("no server, no token list");
        assert_eq!(failure.reason, reasons::PLEX_SERVER_NOT_SELECTED);
    }

    /// plex.tv names a shared user but gives no token for them: reading them
    /// from history instead would drop their manually marked plays, so their
    /// read fails. Another shared user on the same list is still read.
    #[tokio::test]
    async fn a_shared_user_listed_without_a_token_fails_alone() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer size="2"><SharedServer userID="2002" /><SharedServer userID="3003" accessToken="shared-token-c" /></MediaContainer>"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0, "offset": 0, "totalSize": 0 }
            })))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .and(header("X-Plex-Token", "shared-token-c"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0 }
            })))
            .expect(1..)
            .mount(&server)
            .await;

        let reader = plex_source(&server)
            .open(&plex_connection(server.uri()))
            .await
            .expect("opens");
        let failure = reader
            .fetch_played_items(PLEX_SHARED_ID)
            .await
            .expect_err("a listed user without a token is not readable");
        assert_eq!(failure.reason, reasons::RESPONSE_UNREADABLE);
        let items = reader
            .fetch_played_items("3003")
            .await
            .expect("the user with a token is still read");
        assert!(items.is_empty());
    }

    /// A body that is not the shared-server list at all fails every
    /// non-owner read, not only one user's.
    #[tokio::test]
    async fn a_shared_server_body_without_its_root_fails_every_shared_read() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"<errors><error code="1001" /></errors>"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let reader = plex_source(&server)
            .open(&plex_connection(server.uri()))
            .await
            .expect("opens");
        for account in [PLEX_SHARED_ID, "3003"] {
            let failure = reader
                .fetch_played_items(account)
                .await
                .expect_err("an unreadable list reads no one but the owner");
            assert_eq!(failure.reason, reasons::RESPONSE_UNREADABLE);
        }
    }

    /// The owner's token can read every account's history. A server that
    /// ignored the `accountID` filter would attribute other people's plays to
    /// this account, so a row naming another account fails the read.
    #[tokio::test]
    async fn a_history_row_for_another_account_is_a_filter_not_honoured() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"<MediaContainer size="0"></MediaContainer>"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("accountID", PLEX_SHARED_ID))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 2, "offset": 0, "totalSize": 2, "Metadata": [
                    { "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_500_000 },
                    { "ratingKey": "201", "type": "episode", "accountID": 1, "viewedAt": 1_700_100_000 }
                ] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101,201"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let failure = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect_err("another account's play is not this account's");
        assert_eq!(failure.reason, reasons::FILTER_NOT_HONOURED);
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

    fn history_rows(first_play: usize, count: usize) -> Vec<Value> {
        (first_play..first_play + count)
            .map(|index| {
                json!({
                    "ratingKey": (index % 3).to_string(),
                    "type": "movie",
                    "accountID": 2002,
                    "viewedAt": 1_700_000_000 + index as i64
                })
            })
            .collect()
    }

    /// History rows repeat items by design, so a server answering every page
    /// from the top is caught by the plays it repeats, offset or not.
    #[tokio::test]
    async fn a_plex_history_page_answered_from_the_top_again_is_not_honouring_paging() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_no_shared_server_tokens(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": SIGNAL_PAGE_SIZE,
                    "totalSize": SIGNAL_PAGE_SIZE * 2,
                    "Metadata": history_rows(0, SIGNAL_PAGE_SIZE)
                }
            })))
            .mount(&server)
            .await;

        let error = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect_err("a repeated page is not proof of completeness");
        assert_eq!(error.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// History pages that carry no `offset` but do advance are a complete
    /// read: python-plexapi pages history the same way, by start and
    /// totalSize alone.
    #[tokio::test]
    async fn plex_history_pages_without_an_offset_read_whole_when_they_advance() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_no_shared_server_tokens(&server).await;
        for (start, rows) in [
            (0, history_rows(0, SIGNAL_PAGE_SIZE)),
            (SIGNAL_PAGE_SIZE, history_rows(SIGNAL_PAGE_SIZE, 2)),
        ] {
            Mock::given(method("GET"))
                .and(path("/status/sessions/history/all"))
                .and(header("X-Plex-Container-Start", start.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "MediaContainer": {
                        "size": rows.len(),
                        "totalSize": SIGNAL_PAGE_SIZE + 2,
                        "Metadata": rows
                    }
                })))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/library/metadata/0,1,2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0 }
            })))
            .mount(&server)
            .await;

        plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect("an advancing history needs no offset echo");
    }

    /// A history row that names no account cannot show the account filter
    /// held, so it fails like a row naming someone else.
    #[tokio::test]
    async fn a_history_row_without_an_account_is_a_filter_not_honoured() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_no_shared_server_tokens(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 2, "offset": 0, "totalSize": 2, "Metadata": [
                    { "ratingKey": "101", "type": "movie", "accountID": 2002, "viewedAt": 1_700_500_000 },
                    { "ratingKey": "102", "type": "movie", "viewedAt": 1_700_100_000 }
                ] }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101,102"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let failure = plex_source(&server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect_err("an unattributed row is not this account's");
        assert_eq!(failure.reason, reasons::FILTER_NOT_HONOURED);
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
            r#"<MediaContainer><SharedServer userID="2002" accessToken="token-a"><Section key="1" /></SharedServer><SharedServer userID="3003" accessToken="token-b" /></MediaContainer>"#,
        )
        .expect("readable");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens.get("2002"), Some(&Some("token-a".to_string())));
        assert_eq!(tokens.get("3003"), Some(&Some("token-b".to_string())));
        assert!(shared_server_tokens("<MediaContainer><SharedServer").is_none());
        assert_eq!(
            shared_server_tokens(r#"<MediaContainer size="0" />"#),
            Some(BTreeMap::new())
        );
    }

    /// A user named without a usable token is that user's problem alone: they
    /// are marked unreadable and everyone else keeps their token.
    #[test]
    fn a_shared_user_without_a_usable_token_is_marked_and_the_rest_stand() {
        for (body, untokened) in [
            (
                r#"<MediaContainer><SharedServer userID="2002" accessToken="token-a" /><SharedServer userID="3003" accessToken="" /></MediaContainer>"#,
                "3003",
            ),
            (
                r#"<MediaContainer><SharedServer userID="3003" /><SharedServer userID="2002" accessToken="token-a" /></MediaContainer>"#,
                "3003",
            ),
            (
                r#"<MediaContainer><SharedServer userID="3003" accessToken="   " /><SharedServer userID="2002" accessToken="token-a" /></MediaContainer>"#,
                "3003",
            ),
        ] {
            let tokens = shared_server_tokens(body).expect("the list itself is readable");
            assert_eq!(tokens.get(untokened), Some(&None), "{body}");
            assert_eq!(
                tokens.get("2002"),
                Some(&Some("token-a".to_string())),
                "{body}"
            );
        }
    }

    /// A well-formed body that is not the shared-server list is not "nobody is
    /// shared with".
    #[test]
    fn a_shared_server_body_without_a_media_container_root_is_unreadable() {
        for body in [
            r#"<errors><error code="1001" message="synthetic" /></errors>"#,
            r#"<html><body>synthetic sign-in page</body></html>"#,
            "",
        ] {
            assert!(shared_server_tokens(body).is_none(), "{body:?}");
        }
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

    // ── Live re-check before an action ──────────────────────────────────────

    fn query_value(request: &Request, key: &str) -> Option<String> {
        request
            .url
            .query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    }

    /// Plex history for one item: the owner, a shared user, a user read from
    /// history, and an account no one has linked.
    async fn mount_plex_item_history(server: &wiremock::MockServer, rows: Value) {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("metadataItemID", "101"))
            .and(query_param("sort", "viewedAt:asc"))
            .and(header("X-Plex-Token", PLEX_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": rows.as_array().map_or(0, Vec::len),
                    "totalSize": rows.as_array().map_or(0, Vec::len),
                    "Metadata": rows
                }
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    async fn mount_plex_item_metadata(server: &wiremock::MockServer, token: &str, item: Value) {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/library/metadata/101"))
            .and(header("X-Plex-Token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 1, "Metadata": [item] }
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    async fn mount_shared_token(server: &wiremock::MockServer, token: &str) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/api/servers/plex-machine-1/shared_servers"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"<MediaContainer size="1"><SharedServer userID="2002" accessToken="{token}" /></MediaContainer>"#
            )))
            .mount(server)
            .await;
    }

    async fn plex_recheck(
        server: &wiremock::MockServer,
        items: &[&str],
        participants: &[&str],
    ) -> ReadResult<BTreeMap<String, ProviderItemRecheck>> {
        plex_source(server)
            .open(&plex_connection(server.uri()))
            .await?
            .recheck_items(
                &items.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                &participants
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>(),
            )
            .await
    }

    #[tokio::test]
    async fn a_plex_recheck_reads_history_for_every_account_and_each_participants_own_state() {
        let server = wiremock::MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_shared_token(&server, "shared-token-b").await;
        mount_plex_item_history(
            &server,
            json!([
                { "ratingKey": "101", "type": "movie", "accountID": 1, "viewedAt": 1_756_100_000 },
                { "ratingKey": "101", "type": "movie", "accountID": 9009, "viewedAt": 1_756_200_000 },
                { "ratingKey": "101", "type": "movie", "accountID": 3003, "viewedAt": 1_756_300_000 }
            ]),
        )
        .await;
        mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(1, 1_756_100_000)).await;
        mount_plex_item_metadata(&server, "shared-token-b", plex_movie(0, 0)).await;

        let mut rechecks =
            plex_recheck(&server, &["101"], &[PLEX_OWNER_ID, PLEX_SHARED_ID, "3003"])
                .await
                .expect("the item re-checks");
        let recheck = rechecks.remove("101").expect("the item is answered");

        let at = |seconds| DateTime::from_timestamp(seconds, 0);
        assert_eq!(
            recheck.participants,
            vec![
                ProviderItemPlay {
                    external_user_id: PLEX_OWNER_ID.into(),
                    played: true,
                    last_played_at: at(1_756_100_000),
                },
                ProviderItemPlay {
                    external_user_id: PLEX_SHARED_ID.into(),
                    played: false,
                    last_played_at: None,
                },
                // Read from history: their state is their newest row there.
                ProviderItemPlay {
                    external_user_id: "3003".into(),
                    played: true,
                    last_played_at: at(1_756_300_000),
                },
            ]
        );
        // History names the owner as the local account `1`; the play is the
        // owner's, under the plex.tv id their linked user carries.
        let history_accounts = recheck
            .history
            .iter()
            .map(|play| play.external_user_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(history_accounts, [PLEX_OWNER_ID, "9009", "3003"]);
    }

    #[tokio::test]
    async fn a_plex_item_history_naming_another_item_is_not_honouring_the_filter() {
        let server = wiremock::MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(1, 1_756_100_000)).await;
        mount_plex_item_history(
            &server,
            json!([{ "ratingKey": "555", "type": "movie", "accountID": 1001, "viewedAt": 1_756_100_000 }]),
        )
        .await;

        let failure = plex_recheck(&server, &["101"], &[PLEX_OWNER_ID])
            .await
            .expect_err("rows for another item are not this item's history");
        assert_eq!(failure.reason, reasons::FILTER_NOT_HONOURED);
    }

    #[tokio::test]
    async fn a_plex_history_row_without_an_account_fails_the_recheck() {
        let server = wiremock::MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(1, 1_756_100_000)).await;
        mount_plex_item_history(
            &server,
            json!([{ "ratingKey": "101", "type": "movie", "viewedAt": 1_756_100_000 }]),
        )
        .await;

        let failure = plex_recheck(&server, &["101"], &[PLEX_OWNER_ID])
            .await
            .expect_err("a play no one can be credited with cannot be cleared");
        assert_eq!(failure.reason, reasons::ITEMS_UNREADABLE);
    }

    #[tokio::test]
    async fn a_plex_item_gone_from_the_library_fails_the_recheck() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let failure = plex_recheck(&server, &["101"], &[PLEX_OWNER_ID])
            .await
            .expect_err("an item the server no longer lists cannot read as unplayed");
        assert_eq!(failure.reason, reasons::RESPONSE_UNREADABLE);
    }

    /// A shared user the server now refuses for the item, whether as gone
    /// (404) or as forbidden (403, a library no longer shared), has no plays
    /// of it: their answer is "unplayed", and the re-check stands.
    #[tokio::test]
    async fn a_plex_shared_user_the_server_refuses_reads_as_unplayed() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        for status in [404, 403] {
            let server = MockServer::start().await;
            mount_plex_owner(&server).await;
            mount_shared_token(&server, "shared-token-b").await;
            mount_plex_item_history(&server, json!([])).await;
            mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(0, 0)).await;
            Mock::given(method("GET"))
                .and(path("/library/metadata/101"))
                .and(header("X-Plex-Token", "shared-token-b"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            let recheck = plex_recheck(&server, &["101"], &[PLEX_SHARED_ID])
                .await
                .unwrap_or_else(|failure| panic!("status {status}: {failure}"))
                .remove("101")
                .expect("the item is answered");
            assert_eq!(
                recheck.participants,
                vec![ProviderItemPlay {
                    external_user_id: PLEX_SHARED_ID.into(),
                    played: false,
                    last_played_at: None,
                }],
                "status {status}"
            );
        }
    }

    #[tokio::test]
    async fn a_plex_shared_user_read_that_fails_on_the_server_fails_the_recheck() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_shared_token(&server, "shared-token-b").await;
        mount_plex_item_history(&server, json!([])).await;
        mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(0, 0)).await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/101"))
            .and(header("X-Plex-Token", "shared-token-b"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let failure = plex_recheck(&server, &["101"], &[PLEX_SHARED_ID])
            .await
            .expect_err("a server error is not an answer");
        assert_eq!(failure.reason, reasons::REQUEST_REJECTED);
    }

    /// A whole series is re-checked in a bounded number of requests: item
    /// state in batches of `LOOKUP_BATCH`, and history one request per item,
    /// since Plex history filters on a single item.
    #[tokio::test]
    async fn a_plex_series_recheck_batches_its_state_reads() {
        use wiremock::matchers::{method, path, path_regex};
        use wiremock::{Mock, MockServer};

        const EPISODES: usize = 200;
        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/library/metadata/[0-9,]+$"))
            .respond_with(GeneratedMetadata)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": 0, "totalSize": 0, "Metadata": [] }
            })))
            .mount(&server)
            .await;

        let keys: Vec<String> = (1..=EPISODES).map(|key| key.to_string()).collect();
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let rechecks = plex_recheck(&server, &key_refs, &[PLEX_OWNER_ID])
            .await
            .expect("the series re-checks");
        assert_eq!(rechecks.len(), EPISODES);

        let requests = server.received_requests().await.expect("recorded");
        let metadata = requests
            .iter()
            .filter(|request| request.url.path().starts_with("/library/metadata/"))
            .count();
        let history = requests
            .iter()
            .filter(|request| request.url.path() == "/status/sessions/history/all")
            .count();
        assert_eq!(metadata, EPISODES.div_ceil(LOOKUP_BATCH));
        assert_eq!(history, EPISODES);
    }

    /// `/Users/{id}/Items?Ids=…` for a Jellyfin or Emby user: each id asked for
    /// that the user can see, played when `played` names it.
    fn items_by_id(
        played: &'static [(&'static str, &'static str)],
        hidden: &'static [&'static str],
    ) -> impl Fn(&Request) -> ResponseTemplate + Send + Sync {
        move |request: &Request| {
            let items = query_value(request, "Ids")
                .unwrap_or_default()
                .split(',')
                .filter(|id| !id.is_empty() && !hidden.contains(id))
                .map(
                    |id| match played.iter().find(|(played_id, _)| *played_id == id) {
                        Some((_, at)) => json!({
                            "Id": id,
                            "Type": "Episode",
                            "UserData": { "Played": true, "PlayCount": 1, "LastPlayedDate": at }
                        }),
                        None => json!({
                            "Id": id,
                            "Type": "Episode",
                            "UserData": { "Played": false, "PlayCount": 0 }
                        }),
                    },
                )
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({ "Items": items }))
        }
    }

    async fn mount_users(server: &wiremock::MockServer, prefix: &str, users: &[&str]) {
        use wiremock::Mock;
        use wiremock::matchers::{method, path};
        Mock::given(method("GET"))
            .and(path(format!("{prefix}/Users")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(
                users.iter().map(|id| json!({ "Id": id })).collect::<Vec<_>>()
            )))
            .mount(server)
            .await;
    }

    async fn mount_user_items(
        server: &wiremock::MockServer,
        prefix: &str,
        user: &str,
        responder: impl wiremock::Respond + 'static,
    ) {
        use wiremock::Mock;
        use wiremock::matchers::{method, path};
        Mock::given(method("GET"))
            .and(path(format!("{prefix}/Users/{user}/Items")))
            .respond_with(responder)
            .mount(server)
            .await;
    }

    async fn jellyfin_recheck(
        server: &wiremock::MockServer,
        items: &[&str],
        participants: &[&str],
    ) -> ReadResult<BTreeMap<String, ProviderItemRecheck>> {
        let mut connection = connection_fixture();
        connection.base_url = server.uri();
        HttpMediaServerSignalSource::new()
            .open(&connection)
            .await?
            .recheck_items(
                &items.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                &participants
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>(),
            )
            .await
    }

    /// Emby keeps no server history, so every listed user's own state stands
    /// in for one: an unlinked user's play is one of the item's plays.
    #[tokio::test]
    async fn an_emby_recheck_reads_every_users_state_for_the_item() {
        let server = wiremock::MockServer::start().await;
        mount_users(
            &server,
            "/emby",
            &["emby-user-1", "emby-user-2", "emby-user-3"],
        )
        .await;
        mount_user_items(
            &server,
            "/emby",
            "emby-user-1",
            items_by_id(&[("emby-item-7", "2026-10-30T08:15:00.0000000Z")], &[]),
        )
        .await;
        mount_user_items(&server, "/emby", "emby-user-2", items_by_id(&[], &[])).await;
        mount_user_items(
            &server,
            "/emby",
            "emby-user-3",
            items_by_id(&[("emby-item-7", "2026-10-31T21:00:00.0000000Z")], &[]),
        )
        .await;

        let recheck = HttpMediaServerSignalSource::new()
            .open(&emby_connection(format!("{}/emby", server.uri())))
            .await
            .expect("opens")
            .recheck_items(
                &["emby-item-7".to_string()],
                &["emby-user-1".to_string(), "emby-user-2".to_string()],
            )
            .await
            .expect("the item re-checks")
            .remove("emby-item-7")
            .expect("the item is answered");

        assert_eq!(
            recheck.participants,
            vec![
                ProviderItemPlay {
                    external_user_id: "emby-user-1".into(),
                    played: true,
                    last_played_at: parse_timestamp("2026-10-30T08:15:00Z"),
                },
                ProviderItemPlay {
                    external_user_id: "emby-user-2".into(),
                    played: false,
                    last_played_at: None,
                },
            ]
        );
        assert_eq!(
            recheck.history,
            vec![
                ProviderItemPlay {
                    external_user_id: "emby-user-1".into(),
                    played: true,
                    last_played_at: parse_timestamp("2026-10-30T08:15:00Z"),
                },
                ProviderItemPlay {
                    external_user_id: "emby-user-3".into(),
                    played: true,
                    last_played_at: parse_timestamp("2026-10-31T21:00:00Z"),
                },
            ]
        );
    }

    #[tokio::test]
    async fn an_emby_item_answer_without_user_data_fails_the_recheck() {
        let server = wiremock::MockServer::start().await;
        mount_users(&server, "/emby", &["emby-user-1"]).await;
        mount_user_items(&server, "/emby", "emby-user-1", |_: &Request| {
            ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{ "Id": "emby-item-7", "Type": "Movie" }]
            }))
        })
        .await;

        let failure = HttpMediaServerSignalSource::new()
            .open(&emby_connection(format!("{}/emby", server.uri())))
            .await
            .expect("opens")
            .recheck_items(&["emby-item-7".to_string()], &["emby-user-1".to_string()])
            .await
            .expect_err("no user data is no evidence the item is unplayed");
        assert_eq!(failure.reason, reasons::RESPONSE_UNREADABLE);
    }

    #[tokio::test]
    async fn a_jellyfin_item_no_user_can_see_fails_the_recheck() {
        let server = wiremock::MockServer::start().await;
        mount_users(&server, "", &["jf-user-1"]).await;
        mount_user_items(&server, "", "jf-user-1", items_by_id(&[], &["jf-item-9"])).await;

        let failure = jellyfin_recheck(&server, &["jf-item-9"], &["jf-user-1"])
            .await
            .expect_err("an item the server no longer has cannot be shown unplayed");
        assert_eq!(failure.reason, reasons::RESPONSE_UNREADABLE);
    }

    /// A user the server refuses for the items (removed: 404; a library they
    /// may no longer see: 403) has no plays of them. Only a refusal of every
    /// read is the admin key's own, and fails.
    #[tokio::test]
    async fn a_jellyfin_user_the_server_refuses_reads_as_unplayed() {
        for status in [404, 403] {
            let server = wiremock::MockServer::start().await;
            mount_users(&server, "", &["jf-user-1", "jf-user-2"]).await;
            mount_user_items(&server, "", "jf-user-1", items_by_id(&[], &[])).await;
            mount_user_items(&server, "", "jf-user-2", ResponseTemplate::new(status)).await;

            let recheck = jellyfin_recheck(&server, &["jf-item-9"], &["jf-user-1", "jf-user-2"])
                .await
                .unwrap_or_else(|failure| panic!("status {status}: {failure}"))
                .remove("jf-item-9")
                .expect("the item is answered");
            assert!(
                recheck.participants.iter().all(|state| !state.played),
                "status {status}: {:?}",
                recheck.participants
            );
            assert_eq!(recheck.participants.len(), 2);
            assert!(recheck.history.is_empty());
        }

        let server = wiremock::MockServer::start().await;
        mount_users(&server, "", &["jf-user-1", "jf-user-2"]).await;
        mount_user_items(&server, "", "jf-user-1", ResponseTemplate::new(403)).await;
        mount_user_items(&server, "", "jf-user-2", ResponseTemplate::new(403)).await;
        let failure = jellyfin_recheck(&server, &["jf-item-9"], &["jf-user-1"])
            .await
            .expect_err("every read refused is the credential's refusal");
        assert_eq!(failure.reason, reasons::ACCESS_DENIED);
    }

    /// Any user's read failing for another reason fails the re-check, even a
    /// user no Scryer user is linked to: their plays count for the anonymous
    /// facts.
    #[tokio::test]
    async fn a_jellyfin_user_read_that_fails_on_the_server_fails_the_recheck() {
        let server = wiremock::MockServer::start().await;
        mount_users(&server, "", &["jf-user-1", "jf-unlinked"]).await;
        mount_user_items(&server, "", "jf-user-1", items_by_id(&[], &[])).await;
        mount_user_items(&server, "", "jf-unlinked", ResponseTemplate::new(500)).await;

        let failure = jellyfin_recheck(&server, &["jf-item-9"], &["jf-user-1"])
            .await
            .expect_err("a server error is not an answer");
        assert_eq!(failure.reason, reasons::REQUEST_REJECTED);
    }

    #[tokio::test]
    async fn a_jellyfin_series_recheck_batches_its_reads_per_user() {
        const EPISODES: usize = 200;
        let server = wiremock::MockServer::start().await;
        mount_users(&server, "", &["jf-user-1", "jf-user-2"]).await;
        mount_user_items(&server, "", "jf-user-1", items_by_id(&[], &[])).await;
        mount_user_items(&server, "", "jf-user-2", items_by_id(&[], &[])).await;

        let ids: Vec<String> = (0..EPISODES)
            .map(|index| format!("jf-ep-{index}"))
            .collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let rechecks = jellyfin_recheck(&server, &id_refs, &["jf-user-1"])
            .await
            .expect("the series re-checks");
        assert_eq!(rechecks.len(), EPISODES);

        let requests = server.received_requests().await.expect("recorded").len();
        // The user list, then each user's items in batches.
        assert_eq!(requests, 1 + 2 * EPISODES.div_ceil(LOOKUP_BATCH));
    }

    // ── Paging that never ends, and history that moves ──────────────────────

    #[tokio::test]
    async fn a_listing_that_never_signals_its_end_fails_at_the_page_ceiling() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(GeneratedPages {
                plex: false,
                total: usize::MAX,
                report_total: false,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
                growing_total: false,
            })
            .expect(3)
            .mount(&server)
            .await;
        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let failure = HttpMediaServerSignalSource::new()
            .with_page_ceiling(3)
            .read_played(&connection, "user-123")
            .await
            .expect_err("a listing with no end is not a complete read");
        assert_eq!(failure.reason, reasons::PAGING_NOT_HONOURED);
    }

    #[tokio::test]
    async fn a_plex_history_that_never_signals_its_end_fails_at_the_page_ceiling() {
        use wiremock::matchers::{method, path, path_regex};
        use wiremock::{Mock, MockServer};

        let server = MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_no_shared_server_tokens(&server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(GeneratedPages {
                plex: true,
                total: usize::MAX,
                report_total: false,
                page_limit: SIGNAL_PAGE_SIZE,
                distinct_keys: None,
                growing_total: false,
            })
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/library/metadata/[0-9,]+$"))
            .respond_with(GeneratedMetadata)
            .mount(&server)
            .await;

        let failure = plex_source(&server)
            .with_page_ceiling(3)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
            .expect_err("a history with no end is not a complete read");
        assert_eq!(failure.reason, reasons::PAGING_NOT_HONOURED);
    }

    #[test]
    fn the_page_ceiling_is_far_past_any_real_history() {
        const { assert!(SIGNAL_PAGE_CEILING * SIGNAL_PAGE_SIZE >= 1_000_000) };
    }

    /// A Plex history the server sorts as asked, serving plays as they stand
    /// at each request. `arrivals` are played between the first page and the
    /// second, the newest plays on the server.
    struct LiveHistory {
        plays: std::sync::Mutex<Vec<Value>>,
        arrivals: std::sync::Mutex<Vec<Value>>,
        requests: std::sync::atomic::AtomicUsize,
    }

    impl wiremock::Respond for LiveHistory {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let call = self
                .requests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut plays = self.plays.lock().expect("plays");
            if call == 1 {
                plays.append(&mut self.arrivals.lock().expect("arrivals"));
            }
            let header = |name: &str| {
                request
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<usize>().ok())
            };
            let start = header("X-Plex-Container-Start").unwrap_or(0);
            let size = header("X-Plex-Container-Size").unwrap_or(usize::MAX);
            let mut sorted = plays.clone();
            sorted.sort_by_key(|play| play["viewedAt"].as_i64());
            if query_value(request, "sort").as_deref() == Some("viewedAt:desc") {
                sorted.reverse();
            }
            let page = sorted
                .iter()
                .skip(start)
                .take(size)
                .cloned()
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": page.len(),
                    "totalSize": sorted.len(),
                    "offset": start,
                    "Metadata": page
                }
            }))
        }
    }

    fn history_play(index: usize) -> Value {
        json!({
            "ratingKey": (index + 1).to_string(),
            "type": "movie",
            "accountID": 2002,
            "viewedAt": 1_700_000_000 + index as i64
        })
    }

    async fn read_history_of(
        server: &wiremock::MockServer,
        history: impl wiremock::Respond + 'static,
    ) -> ReadResult<Vec<ProviderPlayedItem>> {
        use wiremock::Mock;
        use wiremock::matchers::{method, path, path_regex};

        mount_plex_owner(server).await;
        mount_no_shared_server_tokens(server).await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .respond_with(history)
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/library/metadata/[0-9,]+$"))
            .respond_with(GeneratedMetadata)
            .mount(server)
            .await;
        plex_source(server)
            .read_played(&plex_connection(server.uri()), PLEX_SHARED_ID)
            .await
    }

    /// A play that lands between two pages joins the end of an oldest-first
    /// listing: the read completes with every play, the new one included.
    #[tokio::test]
    async fn a_plex_play_arriving_mid_read_is_read_with_the_rest() {
        let initial = SIGNAL_PAGE_SIZE + 100;
        let server = wiremock::MockServer::start().await;
        let items = read_history_of(
            &server,
            LiveHistory {
                plays: std::sync::Mutex::new((0..initial).map(history_play).collect()),
                arrivals: std::sync::Mutex::new(vec![history_play(initial)]),
                requests: std::sync::atomic::AtomicUsize::new(0),
            },
        )
        .await
        .expect("a play landing mid-read does not break the read");
        assert_eq!(items.len(), initial + 1);
    }

    /// A row a later page repeats, from a page that starts one row early, is
    /// read once: the distinct rows still reach the reported total, so the
    /// read completes without the repeat counting as a second play.
    #[tokio::test]
    async fn a_plex_history_row_a_later_page_repeats_is_read_once() {
        let total = SIGNAL_PAGE_SIZE + 100;
        let server = wiremock::MockServer::start().await;
        let overlapping = move |request: &Request| {
            let start = request
                .headers
                .get("X-Plex-Container-Start")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            // The second page starts one row early.
            let from = if start == 0 { 0 } else { start - 1 };
            let rows = (from..total.min(from + SIGNAL_PAGE_SIZE))
                .map(history_play)
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": { "size": rows.len(), "totalSize": total, "Metadata": rows }
            }))
        };
        let items = read_history_of(&server, overlapping)
            .await
            .expect("an overlap between pages is not a failed read");
        assert_eq!(items.len(), total);
        assert!(items.iter().all(|item| item.play_count == 1));
    }

    /// A play inserted before the cursor mid-read shifts the rows after it
    /// forward: the next page repeats one row, and the inserted play is never
    /// read. The distinct rows fall short of the grown total, so the read
    /// fails rather than passing that play over.
    #[tokio::test]
    async fn a_plex_play_inserted_before_the_cursor_fails_the_read() {
        let initial = SIGNAL_PAGE_SIZE + 100;
        let mut backfilled = history_play(initial);
        backfilled["viewedAt"] = json!(1_600_000_000);
        let server = wiremock::MockServer::start().await;
        let failure = read_history_of(
            &server,
            LiveHistory {
                plays: std::sync::Mutex::new((0..initial).map(history_play).collect()),
                arrivals: std::sync::Mutex::new(vec![backfilled]),
                requests: std::sync::atomic::AtomicUsize::new(0),
            },
        )
        .await
        .expect_err("a play the read passed over is not a complete history");
        assert_eq!(failure.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// History rows in `viewedAt` order, except that the two rows either side
    /// of the first page boundary share a time and swap places after the
    /// first request, as a server free to order ties either way may.
    struct SwappingHistory {
        rows: Vec<Value>,
        requests: std::sync::atomic::AtomicUsize,
    }

    impl SwappingHistory {
        fn new(rows: Vec<Value>) -> Self {
            let mut rows = rows;
            let tie = rows[SIGNAL_PAGE_SIZE - 1]["viewedAt"].clone();
            rows[SIGNAL_PAGE_SIZE]["viewedAt"] = tie;
            Self {
                rows,
                requests: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl wiremock::Respond for SwappingHistory {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let call = self
                .requests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut rows = self.rows.clone();
            if call > 0 {
                rows.swap(SIGNAL_PAGE_SIZE - 1, SIGNAL_PAGE_SIZE);
            }
            let start = request
                .headers
                .get("X-Plex-Container-Start")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let page = rows
                .iter()
                .skip(start)
                .take(SIGNAL_PAGE_SIZE)
                .cloned()
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": page.len(),
                    "totalSize": rows.len(),
                    "offset": start,
                    "Metadata": page
                }
            }))
        }
    }

    /// Two plays sharing a time at the page boundary swap order between
    /// requests: the second page repeats one and the other is never read.
    /// Every page is well formed, but the distinct rows fall one short of the
    /// reported total, and the read fails.
    #[tokio::test]
    async fn plex_plays_swapping_places_at_a_page_boundary_fail_the_read() {
        let total = SIGNAL_PAGE_SIZE + 100;
        let server = wiremock::MockServer::start().await;
        let failure = read_history_of(
            &server,
            SwappingHistory::new((0..total).map(history_play).collect()),
        )
        .await
        .expect_err("a play passed over at the boundary is not a complete history");
        assert_eq!(failure.reason, reasons::PAGING_NOT_HONOURED);
    }

    /// The same swap in one item's history fails the pre-action re-check.
    #[tokio::test]
    async fn plex_item_plays_swapping_places_at_a_page_boundary_fail_the_recheck() {
        use wiremock::Mock;
        use wiremock::matchers::{method, path, query_param};

        let total = SIGNAL_PAGE_SIZE + 100;
        let server = wiremock::MockServer::start().await;
        mount_plex_owner(&server).await;
        mount_plex_item_metadata(&server, PLEX_TOKEN, plex_movie(1, 1_756_100_000)).await;
        let rows = (0..total)
            .map(|index| {
                json!({
                    "ratingKey": "101",
                    "type": "movie",
                    "accountID": 9009,
                    "viewedAt": 1_700_000_000 + index as i64
                })
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("metadataItemID", "101"))
            .respond_with(SwappingHistory::new(rows))
            .mount(&server)
            .await;

        let failure = plex_recheck(&server, &["101"], &[PLEX_OWNER_ID])
            .await
            .expect_err("a play passed over at the boundary cannot clear the item");
        assert_eq!(failure.reason, reasons::PAGING_NOT_HONOURED);
    }
}
