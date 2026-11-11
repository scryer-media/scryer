//! "From your watchlist": the titles the signed-in viewer has said they want
//! to watch, that the library does not have and nobody has requested yet.
//!
//! The section is the viewer's own. It is composed per request from the
//! viewer's personal list memberships and never from another member's. A
//! member is shown as the discovery item this viewer may already see when one
//! exists, so it is the same card, with the same side panel, as on any other
//! rail; a member no stored item names still gets a plain card built from its
//! list row, so the watchlist never silently loses a title.

use super::*;
use crate::lists::ListSubscriptionQuery;
use crate::ports::{DiscoveryExternalIdRecord, DiscoveryHomeFilters};
use scryer_domain::{ListMembership, ListMembershipState, ListSubscription};

pub(super) const DISCOVERY_WATCHLIST_SECTION_ID: &str = "from_your_watchlist";
pub(super) const DISCOVERY_WATCHLIST_SECTION_TYPE: &str = "FROM_YOUR_WATCHLIST";
const DISCOVERY_WATCHLIST_SECTION_TITLE: &str = "From Your Watchlist";

/// Target keys resolved per storage query, so a long watchlist never builds
/// one statement past a database's bound-parameter limit.
const DISCOVERY_WATCHLIST_KEY_CHUNK: usize = 400;

/// `source_run_kind` of a card built from a list row rather than a stored
/// discovery item.
const DISCOVERY_WATCHLIST_ROW_RUN_KIND: &str = "watchlist";

/// One personal list source that is a member's "want to watch" list.
struct DiscoveryWatchlistSource {
    provider: &'static str,
    source_type: &'static str,
    /// A parameter the source must carry, and the values (any case) that
    /// make it the watch-later list.
    param: Option<(&'static str, &'static [&'static str])>,
}

/// Every shipped list source that holds a member's watch-later list. Plex's
/// `watchlist_rss` is left out on purpose: it is a public feed URL that may
/// be anyone's and is read without the member's account.
const DISCOVERY_WATCHLIST_SOURCES: &[DiscoveryWatchlistSource] = &[
    // Plex: the linked member's own watchlist.
    DiscoveryWatchlistSource {
        provider: "plex",
        source_type: "watchlist",
        param: None,
    },
    // Trakt: the linked member's watchlist.
    DiscoveryWatchlistSource {
        provider: "trakt",
        source_type: "watchlist",
        param: None,
    },
    // TMDb: the linked account's watchlist.
    DiscoveryWatchlistSource {
        provider: "tmdb",
        source_type: "watchlist",
        param: None,
    },
    // MDBList: the linked member's watchlist.
    DiscoveryWatchlistSource {
        provider: "mdblist",
        source_type: "watchlist",
        param: None,
    },
    // Simkl: the "plan to watch" status list.
    DiscoveryWatchlistSource {
        provider: "simkl",
        source_type: "plantowatch",
        param: None,
    },
    // MyAnimeList: the "plan to watch" status list.
    DiscoveryWatchlistSource {
        provider: "mal",
        source_type: "status:plan_to_watch",
        param: None,
    },
    // AniList: the status list whose `status` is planning.
    DiscoveryWatchlistSource {
        provider: "anilist",
        source_type: "status",
        param: Some(("status", &["planning"])),
    },
];

pub(super) fn discovery_watchlist_source_matches(subscription: &ListSubscription) -> bool {
    let source = &subscription.source;
    DISCOVERY_WATCHLIST_SOURCES.iter().any(|candidate| {
        source
            .provider
            .trim()
            .eq_ignore_ascii_case(candidate.provider)
            && source
                .source_type
                .trim()
                .eq_ignore_ascii_case(candidate.source_type)
            && candidate.param.is_none_or(|(key, values)| {
                source.params.get(key).is_some_and(|value| {
                    values
                        .iter()
                        .any(|expected| value.trim().eq_ignore_ascii_case(expected))
                })
            })
    })
}

/// One title the viewer wants to watch, whatever source said so.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DiscoveryWatchlistMember {
    pub(super) kind: MediaFacet,
    pub(super) external_ids: Vec<ExternalId>,
    pub(super) display_title: Option<String>,
    pub(super) year: Option<i32>,
    pub(super) smg_title_id: Option<i64>,
    /// When Scryer's list poller first saw the row on the viewer's list.
    pub(super) first_seen_at: DateTime<Utc>,
    /// The source's own position, when it has one.
    pub(super) rank: Option<i64>,
    /// An operator excluded this title from the viewer's lists, so the rail
    /// must not offer it again.
    pub(super) declined: bool,
}

impl DiscoveryWatchlistMember {
    fn from_row(row: ListMembership) -> Self {
        Self {
            declined: row.state == ListMembershipState::Excluded,
            kind: row.kind,
            external_ids: row.external_ids,
            display_title: row.display_title,
            year: row.year,
            smg_title_id: row.smg_title_id,
            first_seen_at: row.first_seen_at,
            rank: row.rank,
        }
    }
}

/// The watchlist section and the discovery items it claims away from the
/// "because you like" rails.
#[derive(Debug, Default)]
pub(super) struct DiscoveryWatchlistComposition {
    pub(super) section: Option<DiscoverySectionResult>,
    /// Identity keys of every resolved watchlist member that is not owned,
    /// whether or not it fit in the section.
    pub(super) claimed_item_keys: HashSet<String>,
    /// The pending requests the composition read, for a caller in the same
    /// request that needs them too; `None` when it read none.
    pub(super) pending_requests: Option<Vec<scryer_domain::MediaRequest>>,
}

/// What a composition decides with, besides the members and their items.
pub(super) struct DiscoveryWatchlistRules<'a> {
    pub(super) owned_visibility: &'a CatalogOwnedVisibility,
    pub(super) pending_requests: &'a [scryer_domain::MediaRequest],
    /// Lowercased identity keys of the resolved items the home filter bar
    /// admits; `None` when no filter is set.
    pub(super) admitted_item_keys: Option<&'a HashSet<String>>,
    pub(super) filters: &'a DiscoveryHomeFilters,
    /// The media kinds a card built from a list row may have: the viewer's
    /// visible kinds narrowed by the media-type chips.
    pub(super) row_card_media_kinds: &'a [String],
    pub(super) include_unresolved: bool,
    pub(super) limit: usize,
}

impl AppUseCase {
    /// The viewer's watchlist members across every source, or `None` when the
    /// viewer has no watchlist at all or may not use personal lists. This is
    /// the one place a member source is read: a further source (media-server
    /// favourites, say) appends its members here and everything downstream is
    /// shared.
    pub(super) async fn discovery_watchlist_members(
        &self,
        actor: &User,
    ) -> AppResult<Option<Vec<DiscoveryWatchlistMember>>> {
        if self.personal_lists_refused_by_policy(actor).await? {
            return Ok(None);
        }
        let subscriptions = self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery {
                enabled: Some(true),
                ..ListSubscriptionQuery::personal_for(actor.id.clone())
            })
            .await?
            .into_iter()
            .filter(|subscription| {
                subscription.owner_user_id == actor.id
                    && subscription.is_personal()
                    && subscription.enabled
                    && subscription.credential_id.is_some()
                    && discovery_watchlist_source_matches(subscription)
            })
            .collect::<Vec<_>>();
        if subscriptions.is_empty() {
            return Ok(None);
        }

        let subscription_ids = subscriptions
            .iter()
            .map(|subscription| subscription.id.clone())
            .collect::<Vec<_>>();
        let mut members = self
            .services
            .lists
            .memberships
            .list_active_by_subscriptions(&subscription_ids)
            .await?
            .into_iter()
            .filter(|row| {
                row.left_at.is_none()
                    // In a library somewhere, possibly one this viewer
                    // cannot see: not the watchlist's to offer.
                    && !matches!(
                        row.state,
                        ListMembershipState::InLibrary | ListMembershipState::Added
                    )
            })
            .map(DiscoveryWatchlistMember::from_row)
            .collect::<Vec<_>>();
        sort_discovery_watchlist_members(&mut members);
        Ok(Some(members))
    }

    /// The viewer's watchlist section and claims. `limit` caps only the cards
    /// rendered: claims and `total_count` always cover every member, so a
    /// see-all view passes `usize::MAX` to get the whole rail.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn discovery_watchlist_composition(
        &self,
        actor: &User,
        visibility: &DiscoveryVisibility,
        owned_library_ids: &[String],
        state: &crate::ports::DiscoverySyncStateRecord,
        can_view_personalized: bool,
        filters: &DiscoveryHomeFilters,
        include_unresolved: bool,
        limit: usize,
    ) -> AppResult<DiscoveryWatchlistComposition> {
        let Some(members) = self.discovery_watchlist_members(actor).await? else {
            return Ok(DiscoveryWatchlistComposition::default());
        };
        let allowed_media_kinds = visibility.sorted_allowed_media_kinds();
        if members.is_empty() || allowed_media_kinds.is_empty() {
            return Ok(DiscoveryWatchlistComposition::default());
        }
        let target_keys = members
            .iter()
            .flat_map(discovery_watchlist_member_target_keys)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let context_run_id = can_view_personalized
            .then(|| state.last_success_generation_id.clone())
            .flatten();
        let storage_query =
            |keys: &[String], home_filters: &DiscoveryHomeFilters| DiscoveryItemsStorageQuery {
                context_run_id: context_run_id.clone(),
                public_run_id: state.last_public_feed_generation_id.clone(),
                readable_library_ids: sorted_discovery_library_ids(
                    &visibility.readable_library_ids,
                ),
                allowed_media_kinds: allowed_media_kinds.clone(),
                filters: DiscoveryItemsQuery {
                    target_keys: keys.to_vec(),
                    include_owned: true,
                    include_unresolved: true,
                    include_public: true,
                    limit: keys.len(),
                    offset: 0,
                    ..DiscoveryItemsQuery::default()
                },
                home_filters: home_filters.clone(),
                limit: keys.len(),
                offset: 0,
                ..DiscoveryItemsStorageQuery::default()
            };

        // Every member resolves first, unfiltered, so a title the filter bar
        // hides is still claimed from the reason rails and never mistaken for
        // one with no stored item.
        let mut items = Vec::new();
        for keys in target_keys.chunks(DISCOVERY_WATCHLIST_KEY_CHUNK) {
            items.extend(
                self.services
                    .library
                    .discovery
                    .query_discovery_items_uncounted(&storage_query(
                        keys,
                        &DiscoveryHomeFilters::default(),
                    ))
                    .await?,
            );
        }

        let admitted_item_keys = if *filters == DiscoveryHomeFilters::default() {
            None
        } else {
            // A sync that tombstones an item between the two queries leaves it
            // out of this one load only; the next load reads both consistently.
            let resolved_keys = items
                .iter()
                .map(|item| discovery_item_identity_key(item).to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let mut admitted = HashSet::new();
            for keys in resolved_keys.chunks(DISCOVERY_WATCHLIST_KEY_CHUNK) {
                admitted.extend(
                    self.services
                        .library
                        .discovery
                        .query_discovery_item_identity_keys(&storage_query(keys, filters))
                        .await?
                        .iter()
                        .map(|key| key.to_ascii_lowercase()),
                );
            }
            Some(admitted)
        };

        // Ownership is needed for exactly the members and the items they
        // resolved to, so it reads only the titles those can name.
        let owned_visibility = self
            .owned_visibility_for_targets(
                owned_library_ids,
                target_keys.iter().map(|key| (key.as_str(), None)).chain(
                    items
                        .iter()
                        .map(|item| (item.target_key.as_str(), item.resolved_title_id.as_deref())),
                ),
            )
            .await?;

        // The same pending requests the section pages' "hide requested"
        // reads: every requester's, in the libraries the viewer can request
        // into or read.
        let pending_requests = self.discovery_pending_requests(visibility).await?;

        let row_card_media_kinds = discovery_watchlist_row_card_media_kinds(visibility, filters);
        let mut composition = compose_discovery_watchlist(
            &members,
            items,
            &DiscoveryWatchlistRules {
                owned_visibility: &owned_visibility,
                pending_requests: &pending_requests,
                admitted_item_keys: admitted_item_keys.as_ref(),
                filters,
                row_card_media_kinds: &row_card_media_kinds,
                include_unresolved,
                limit,
            },
        );
        composition.pending_requests = Some(pending_requests);
        Ok(composition)
    }

    /// The card the watchlist rail builds from list rows for `target_key`, for
    /// the side panel and the add or request action, which read a card back by
    /// its key. It answers only the viewer's own watchlist and applies the
    /// rail's gates for that viewer (the personalized switch, unresolved cards,
    /// visible media kinds, ownership, a title every row declined), so a card
    /// the rail would never show has no detail either. A pending request does
    /// not hide it, so the panel still opens on a title just requested.
    pub(super) async fn discovery_watchlist_row_card_detail(
        &self,
        actor: &User,
        visibility: &DiscoveryVisibility,
        target_key: &str,
        include_unresolved: bool,
    ) -> AppResult<Option<DiscoveryItemRecord>> {
        if !include_unresolved || !self.personalized_discovery_enabled().await? {
            return Ok(None);
        }
        let Some(members) = self.discovery_watchlist_members(actor).await? else {
            return Ok(None);
        };
        let target_key = target_key.trim().to_ascii_lowercase();
        let titles = group_discovery_watchlist_members(&members);
        let Some(title) = titles
            .iter()
            .find(|title| title.target_keys.contains(&target_key))
        else {
            return Ok(None);
        };
        let Some(row_key) = title.row_card_target_key() else {
            return Ok(None);
        };
        let media_kind = discovery_media_kind_for_facet(title.kind().clone());
        if title.declined
            || title.display_title().is_none()
            || !visibility
                .sorted_allowed_media_kinds()
                .iter()
                .any(|allowed| allowed == media_kind)
        {
            return Ok(None);
        }
        let owned_library_ids = &visibility.owned_library_ids;
        let owned_visibility = self
            .owned_visibility_for_targets(
                &sorted_discovery_library_ids(owned_library_ids),
                title.target_keys.iter().map(|key| (key.as_str(), None)),
            )
            .await?;
        if title
            .target_keys
            .iter()
            .any(|key| owned_visibility.target_key_is_owned(key))
        {
            return Ok(None);
        }
        Ok(Some(discovery_watchlist_row_card(title, row_key)))
    }
}

/// The provider's own order leads (ranked rows before unranked ones); rows
/// the provider does not rank, or ranks alike, put the ones Scryer saw first
/// most recently ahead.
pub(super) fn sort_discovery_watchlist_members(members: &mut [DiscoveryWatchlistMember]) {
    members.sort_by(|left, right| {
        match (left.rank, right.rank) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| right.first_seen_at.cmp(&left.first_seen_at))
    });
}

/// Every discovery target key a member's external ids could name, lowercased
/// the way storage compares them.
pub(super) fn discovery_watchlist_member_target_keys(
    member: &DiscoveryWatchlistMember,
) -> Vec<String> {
    discovery_target_keys_for_external_ids(&member.external_ids, &member.kind)
}

fn discovery_target_keys_for_external_ids(
    external_ids: &[ExternalId],
    facet: &MediaFacet,
) -> Vec<String> {
    let keys = external_ids
        .iter()
        .filter_map(normalized_discovery_external_id)
        .filter_map(|(source, value)| discovery_target_key_for_external_id(&source, &value, facet))
        .map(|key| key.to_ascii_lowercase())
        .collect::<Vec<_>>();
    unique_discovery_text_terms(keys)
}

/// The media kinds a card built from a list row may carry: what the viewer
/// may see, narrowed by the media-type chips.
fn discovery_watchlist_row_card_media_kinds(
    visibility: &DiscoveryVisibility,
    filters: &DiscoveryHomeFilters,
) -> Vec<String> {
    visibility
        .sorted_allowed_media_kinds()
        .into_iter()
        .filter(|media_kind| {
            filters.content_types.is_empty()
                || filters
                    .content_types
                    .iter()
                    .any(|content_type| content_type.trim().eq_ignore_ascii_case(media_kind))
        })
        .collect()
}

fn discovery_facet_for_media_kind(media_kind: &str) -> Option<MediaFacet> {
    match media_kind {
        "movie" => Some(MediaFacet::Movie),
        "series" => Some(MediaFacet::Series),
        "anime" => Some(MediaFacet::Anime),
        _ => None,
    }
}

/// Target keys every pending request names, built with the same id
/// normalisation as the members' and the items', so a request and a card
/// agree on what one title is (a movie request never answers for a series
/// that shares an id number with it).
fn discovery_requested_target_keys(
    pending_requests: &[scryer_domain::MediaRequest],
) -> HashSet<String> {
    pending_requests
        .iter()
        .flat_map(|request| {
            discovery_target_keys_for_external_ids(&request.external_ids, &request.facet)
        })
        .collect()
}

/// Whether a stored item is already requested, judged by the item's own
/// target key and ids.
fn discovery_item_is_requested(
    item: &DiscoveryItemRecord,
    requested_target_keys: &HashSet<String>,
) -> bool {
    if requested_target_keys.contains(&item.target_key.trim().to_ascii_lowercase()) {
        return true;
    }
    let Some(facet) = discovery_item_media_kind(item).and_then(discovery_facet_for_media_kind)
    else {
        return false;
    };
    let item_ids = item
        .external_ids
        .iter()
        .map(|external_id| ExternalId::new(external_id.source.clone(), external_id.id.clone()))
        .collect::<Vec<_>>();
    discovery_target_keys_for_external_ids(&item_ids, &facet)
        .iter()
        .any(|key| requested_target_keys.contains(key))
}

/// One title on the viewer's watchlists: every member row that names it,
/// joined through any target key two rows share, so the same title on two
/// lists is one card whichever ids each list knows it by.
pub(super) struct DiscoveryWatchlistTitle<'a> {
    members: Vec<&'a DiscoveryWatchlistMember>,
    /// Every target key the rows name, in watchlist order.
    target_keys: Vec<String>,
    /// Every row of the title was declined.
    declined: bool,
}

impl DiscoveryWatchlistTitle<'_> {
    fn kind(&self) -> &MediaFacet {
        &self.members[0].kind
    }

    fn display_title(&self) -> Option<&str> {
        self.members
            .iter()
            .filter_map(|member| member.display_title.as_deref().map(str::trim))
            .find(|title| !title.is_empty())
    }

    fn year(&self) -> Option<i32> {
        self.members.iter().find_map(|member| member.year)
    }

    /// The key a card built from the rows goes by: tmdb, then tvdb, then imdb,
    /// so the title keys alike whichever list named it first. `None` when the
    /// rows carry none of the three, because a request cannot add a title from
    /// any other id; an AniList-, MyAnimeList- or AniDB-only row waits until
    /// SMG knows the title and a stored discovery item names it.
    fn row_card_target_key(&self) -> Option<&str> {
        ["tmdb:", "tvdb:", "imdb:"]
            .iter()
            .find_map(|prefix| self.target_keys.iter().find(|key| key.starts_with(prefix)))
            .map(String::as_str)
    }
}

/// The viewer's watchlist titles in watchlist order (a title sits where its
/// first row does). A row with no usable id is a title of its own.
pub(super) fn group_discovery_watchlist_members(
    members: &[DiscoveryWatchlistMember],
) -> Vec<DiscoveryWatchlistTitle<'_>> {
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }

    let member_keys = members
        .iter()
        .map(discovery_watchlist_member_target_keys)
        .collect::<Vec<_>>();
    let mut parent = (0..members.len()).collect::<Vec<_>>();
    let mut key_owner = HashMap::<&str, usize>::new();
    for (index, keys) in member_keys.iter().enumerate() {
        for key in keys {
            match key_owner.get(key.as_str()) {
                Some(&owner) => {
                    let (left, right) = (root(&mut parent, owner), root(&mut parent, index));
                    parent[left.max(right)] = left.min(right);
                }
                None => {
                    key_owner.insert(key, index);
                }
            }
        }
    }

    let mut titles = Vec::<DiscoveryWatchlistTitle<'_>>::new();
    let mut title_for_root = HashMap::<usize, usize>::new();
    for (index, member) in members.iter().enumerate() {
        let slot = *title_for_root
            .entry(root(&mut parent, index))
            .or_insert_with(|| {
                titles.push(DiscoveryWatchlistTitle {
                    members: Vec::new(),
                    target_keys: Vec::new(),
                    declined: true,
                });
                titles.len() - 1
            });
        let title = &mut titles[slot];
        title.members.push(member);
        title.declined &= member.declined;
        for key in &member_keys[index] {
            if !title.target_keys.contains(key) {
                title.target_keys.push(key.clone());
            }
        }
    }
    titles
}

/// Pure composition. Each watchlist title resolves to the visible discovery
/// item one of its ids names, else to a card built from its list rows.
/// Resolved cards lead and row cards follow, each in watchlist order. Owned
/// titles leave the watchlist entirely; requested, declined and filtered ones
/// stay claimed (they are still on the watchlist and still unowned) but are
/// not offered.
pub(super) fn compose_discovery_watchlist(
    members: &[DiscoveryWatchlistMember],
    items: Vec<DiscoveryItemRecord>,
    rules: &DiscoveryWatchlistRules<'_>,
) -> DiscoveryWatchlistComposition {
    let items_by_key = items
        .into_iter()
        .map(|item| {
            (
                discovery_item_identity_key(&item).to_ascii_lowercase(),
                item,
            )
        })
        .collect::<HashMap<_, _>>();
    let requested_target_keys = discovery_requested_target_keys(rules.pending_requests);
    let titles = group_discovery_watchlist_members(members);

    enum Card<'a> {
        Stored(&'a DiscoveryItemRecord),
        Row(String),
    }
    let card_for = |title: &DiscoveryWatchlistTitle<'_>| -> Option<Card<'_>> {
        if let Some(item) = title
            .target_keys
            .iter()
            .find_map(|key| items_by_key.get(key))
        {
            return Some(Card::Stored(item));
        }
        title.display_title()?;
        title
            .row_card_target_key()
            .map(|key| Card::Row(key.to_string()))
    };
    let card_key = |card: &Card<'_>| match card {
        Card::Stored(item) => discovery_item_identity_key(item).to_ascii_lowercase(),
        Card::Row(key) => key.clone(),
    };

    // A title is offered unless every one of its rows was declined, including
    // two titles the rows keep apart that resolve to one stored item.
    let wanted_keys = titles
        .iter()
        .filter(|title| !title.declined)
        .filter_map(|title| card_for(title).map(|card| card_key(&card)))
        .collect::<HashSet<_>>();

    let mut composition = DiscoveryWatchlistComposition::default();
    let mut stored_cards = Vec::new();
    let mut row_cards = Vec::new();
    let mut seen = HashSet::new();
    let mut left_out = 0usize;
    for title in &titles {
        let Some(card) = card_for(title) else {
            left_out += 1;
            continue;
        };
        let key = card_key(&card);
        if !seen.insert(key.clone()) {
            continue;
        }
        let declined = !wanted_keys.contains(&key);
        match card {
            Card::Stored(item) => {
                if rules.owned_visibility.item_is_owned(item)
                    || (!item.resolved && !rules.include_unresolved)
                {
                    continue;
                }
                composition
                    .claimed_item_keys
                    .insert(discovery_item_identity_key(item).to_string());
                let admitted = rules
                    .admitted_item_keys
                    .is_none_or(|admitted| admitted.contains(&key));
                if declined
                    || !admitted
                    || discovery_item_is_requested(item, &requested_target_keys)
                {
                    continue;
                }
                stored_cards.push(item.clone());
            }
            Card::Row(target_key) => {
                let media_kind = discovery_media_kind_for_facet(title.kind().clone());
                if !rules.include_unresolved
                    || declined
                    || !rules
                        .row_card_media_kinds
                        .iter()
                        .any(|allowed| allowed == media_kind)
                    || title
                        .target_keys
                        .iter()
                        .any(|key| rules.owned_visibility.target_key_is_owned(key))
                    || title
                        .target_keys
                        .iter()
                        .any(|key| requested_target_keys.contains(key))
                    // A list row knows its year and nothing else the filter
                    // bar narrows by, so the year is the only filter it can
                    // honour; like storage, an unknown year passes.
                    || !discovery_watchlist_row_year_admitted(title.year(), rules.filters)
                {
                    continue;
                }
                row_cards.push(discovery_watchlist_row_card(title, &target_key));
            }
        }
    }
    if left_out > 0 {
        debug!(
            member_count = members.len(),
            left_out,
            "watchlist titles with no stored item and no display title or no tmdb, tvdb or imdb id left out"
        );
    }

    let mut offered = stored_cards;
    offered.extend(row_cards);
    let total_count = i64::try_from(offered.len()).unwrap_or(i64::MAX);
    offered.truncate(rules.limit);
    if !offered.is_empty() {
        composition.section = Some(DiscoverySectionResult {
            section_id: DISCOVERY_WATCHLIST_SECTION_ID.to_string(),
            section_type: DISCOVERY_WATCHLIST_SECTION_TYPE.to_string(),
            title: DISCOVERY_WATCHLIST_SECTION_TITLE.to_string(),
            surface: "personalized".to_string(),
            total_count,
            items: offered,
            label_key: None,
        });
    }
    composition
}

fn discovery_watchlist_row_year_admitted(
    year: Option<i32>,
    filters: &DiscoveryHomeFilters,
) -> bool {
    let Some(year) = year else {
        return true;
    };
    filters.minimum_year.is_none_or(|minimum| year >= minimum)
        && filters.maximum_year.is_none_or(|maximum| year <= maximum)
}

/// A card for a title no stored discovery item names, built from its list
/// rows alone: unresolved like any other unresolved discovery item, with every
/// id the rows carry for the request action and no artwork.
fn discovery_watchlist_row_card(
    title: &DiscoveryWatchlistTitle<'_>,
    target_key: &str,
) -> DiscoveryItemRecord {
    let kind = title.kind();
    let media_kind = discovery_media_kind_for_facet(kind.clone());
    let display_title = title.display_title().unwrap_or_default().to_string();
    let mut external_ids = Vec::<DiscoveryExternalIdRecord>::new();
    for (source, value) in title
        .members
        .iter()
        .flat_map(|member| member.external_ids.iter())
        .filter_map(normalized_discovery_external_id)
    {
        let Some(key) = discovery_target_key_for_external_id(&source, &value, kind) else {
            continue;
        };
        let key = key.to_ascii_lowercase();
        if external_ids.iter().any(|existing| existing.key == key) {
            continue;
        }
        external_ids.push(DiscoveryExternalIdRecord {
            source,
            kind: media_kind.to_string(),
            id: value,
            key,
        });
    }
    if let Some(smg_title_id) = title
        .members
        .iter()
        .find_map(|member| member.smg_title_id.filter(|id| *id > 0))
    {
        external_ids.push(DiscoveryExternalIdRecord {
            source: "smg".to_string(),
            kind: media_kind.to_string(),
            id: smg_title_id.to_string(),
            key: format!("smg:{media_kind}:{smg_title_id}"),
        });
    }
    let first_seen_at = title.members[0].first_seen_at;
    DiscoveryItemRecord {
        id: format!("{DISCOVERY_WATCHLIST_ROW_RUN_KIND}:{target_key}"),
        run_id: DISCOVERY_WATCHLIST_ROW_RUN_KIND.to_string(),
        base_generation_id: None,
        source_run_kind: DISCOVERY_WATCHLIST_ROW_RUN_KIND.to_string(),
        section_id: Some(DISCOVERY_WATCHLIST_SECTION_ID.to_string()),
        sort_index: 0,
        target_key: target_key.to_string(),
        target_kind: if *kind == MediaFacet::Movie {
            "movie"
        } else {
            "series"
        }
        .to_string(),
        resolved: false,
        resolved_title_id: None,
        sort_title: Some(display_title.clone()),
        display_title,
        original_title: None,
        year: title.year(),
        poster_path: None,
        poster_url: None,
        background_url: None,
        overview: None,
        content_type: Some(media_kind.to_string()),
        canonical_tags: Vec::new(),
        affinity_signals: Vec::new(),
        is_adult: false,
        content_ratings: Vec::new(),
        rating: None,
        rating_sources: Vec::new(),
        external_ratings: Vec::new(),
        external_ids,
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: Vec::new(),
        best_source: None,
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: None,
        edge_count: None,
        relation_count: None,
        source_subject_count: None,
        rank_score: None,
        recommendation_score: None,
        base_rank: None,
        matched_subject_keys: Vec::new(),
        matched_subject_titles: Vec::new(),
        matched_subject_count: 0,
        library_provenance: Vec::new(),
        tmdb_collection_id: None,
        tmdb_collection_name: None,
        owned_in_input: false,
        studio_slug: None,
        person_ids: Vec::new(),
        facet_terms: Vec::new(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: first_seen_at,
        updated_at: first_seen_at,
    }
}
