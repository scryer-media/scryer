//! List membership facts for maintenance rules.
//!
//! Lists never remove a title. An operator who wants "remove what no list
//! wants any more" writes it as a maintenance rule, and these facts are what
//! that rule reads. They are loaded once per batch of titles and laid over
//! the fact snapshot the builder produced.
//!
//! Personal list names never appear here: a personal list still counts for
//! `lists_on_enabled_list` and `lists_left_all`, but only public lists are
//! named.
//!
//! The watchlist facts read the same memberships, narrowed to the members'
//! own watchlists: the personal subscriptions Discovery treats as "want to
//! watch", attributed to the member who owns each one. They are read only for
//! a rule that names one, and they are unknown for every title while any of
//! those watchlists has not synced recently, because a title missing from a
//! list Scryer has not read lately is not evidence that nobody wants it.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    ListMembership, ListScope, ListSubscription, ListSyncState, MediaFacet, MediaRequestOrigin,
    Title,
};
use scryer_rules::maintenance::{
    EPISODE_SHAPE_MAINTENANCE_FACTS, METADATA_MAINTENANCE_FACTS, MaintenanceInput,
    MaintenanceRatingDoc, Observation, WATCHLIST_MAINTENANCE_FACTS,
};

use super::facts::{MaintenanceTitlePeople, unknown_reason};
use crate::discovery::discovery_watchlist_source_matches;
use crate::lists::ListSubscriptionQuery;
use crate::{AppResult, AppUseCase, TitleEpisodeShape, TitleRatingSummary};

/// What the list store says about one title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleListFacts {
    pub added_by_list: bool,
    pub on_enabled_list: bool,
    pub names: Vec<String>,
    pub left_all: bool,
    pub left_at: Option<DateTime<Utc>>,
    pub last_list_name: Option<String>,
    pub enabled_ids: Vec<String>,
    pub public_count: i64,
    pub personal_count: i64,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub best_rank: Option<i64>,
    pub on_leave: Vec<String>,
    /// `None` when no list has dropped the title.
    pub left_handled: Option<bool>,
    pub membership_states: Vec<String>,
}

/// List facts for a batch, keyed by title id. A title with no entry is on no
/// list and never was.
pub(crate) type ListFactsByTitle = HashMap<String, TitleListFacts>;

/// Where the request that created a title came from. `list_name` is set only
/// for a public list the list store still holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TitleRequestOrigin {
    pub origin: MediaRequestOrigin,
    pub list_name: Option<String>,
}

/// Everything the list and request stores said about a batch. Each is `None`
/// when its store could not be read, so only the facts it feeds read unknown.
#[derive(Clone, Debug)]
pub(crate) struct MaintenanceListFacts {
    pub lists: Option<ListFactsByTitle>,
    pub origins: Option<HashMap<String, TitleRequestOrigin>>,
    pub watchlists: WatchlistLoad,
    /// The stored genres and ratings. They are not list data, but they are
    /// read once per batch for the same subjects and laid over the same
    /// snapshot, so they ride with the list facts.
    pub metadata: MetadataLoad,
    /// The shows' episode-list shapes, read the same way.
    pub episode_shapes: EpisodeShapeLoad,
}

/// The episode-list shapes of a batch's shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EpisodeShapeLoad {
    /// False when no rule being evaluated reads an episode-shape fact.
    pub loaded: bool,
    /// The series and anime titles the read covered; movies are never read.
    pub show_title_ids: HashSet<String>,
    /// `None` when the read failed. A show with no entry has no regular
    /// episode.
    pub shapes: Option<HashMap<String, TitleEpisodeShape>>,
}

/// The stored genres and ratings of a batch. Each map is `None` when its read
/// failed, so only the facts it feeds read unknown.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MetadataLoad {
    /// False when no rule being evaluated reads a metadata fact.
    pub loaded: bool,
    pub genres: Option<HashMap<String, Vec<String>>>,
    pub ratings: Option<HashMap<String, TitleRatingSummary>>,
}

/// The fact groups a rule reads that cost a read of their own. A group no
/// rule reads is never loaded, and its facts stay unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MaintenanceFactLoads {
    pub watchlists: bool,
    pub metadata: bool,
    pub episode_shapes: bool,
}

impl MaintenanceFactLoads {
    /// Every group, for a caller that cannot tell what the rule reads.
    pub(crate) const ALL: Self = Self {
        watchlists: true,
        metadata: true,
        episode_shapes: true,
    };

    /// The groups a rule's source names. A source whose facts cannot be
    /// listed loads every group, so it is never evaluated on facts left
    /// unknown only because they were not read.
    pub(crate) fn for_rule(rego_source: &str, rule_set_id: &str) -> Self {
        match scryer_rules::validation::maintenance_referenced_facts(rego_source, rule_set_id) {
            Ok(facts) => Self {
                watchlists: WATCHLIST_MAINTENANCE_FACTS
                    .iter()
                    .any(|fact| facts.contains(*fact)),
                metadata: METADATA_MAINTENANCE_FACTS
                    .iter()
                    .any(|fact| facts.contains(*fact)),
                episode_shapes: EPISODE_SHAPE_MAINTENANCE_FACTS
                    .iter()
                    .any(|fact| facts.contains(*fact)),
            },
            Err(_) => Self::ALL,
        }
    }
}

/// Who has one title on their watchlist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleWatchlist {
    pub owner_user_ids: BTreeSet<String>,
    pub added_at: Option<DateTime<Utc>>,
    pub origins: BTreeSet<String>,
}

/// The watchlist facts for a batch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum WatchlistLoad {
    /// No rule being evaluated reads a watchlist fact.
    #[default]
    NotLoaded,
    /// A watchlist has not synced recently, or the store could not be read;
    /// every watchlist fact is unknown for this reason.
    Unknown(&'static str),
    /// Keyed by title id. A title with no entry is on no watchlist.
    Loaded(HashMap<String, TitleWatchlist>),
}

/// The oldest successful sync a watchlist may have and still be read: eight
/// of its own polling intervals, and never less than two days, so one missed
/// or rate-limited poll does not hold every rule.
fn watchlist_freshness_window(subscription: &ListSubscription) -> Duration {
    Duration::seconds(subscription.interval_seconds.max(0).saturating_mul(8))
        .max(Duration::hours(48))
}

/// The subscriptions that are a member's own watchlist, read the way
/// Discovery reads them: personal, enabled, holding the member's provider
/// account, and of a source that means "want to watch".
fn is_member_watchlist(subscription: &ListSubscription) -> bool {
    subscription.scope == ListScope::Personal
        && subscription.enabled
        && subscription.sync.state != ListSyncState::Off
        && subscription.credential_id.is_some()
        && discovery_watchlist_source_matches(subscription)
}

/// The reason no watchlist fact can be answered, if any watchlist has not
/// synced successfully within its freshness window.
fn watchlists_unavailable_reason(
    subscriptions: &[&ListSubscription],
    now: DateTime<Utc>,
) -> Option<&'static str> {
    if subscriptions.iter().any(|subscription| {
        matches!(
            subscription.sync.state,
            ListSyncState::New | ListSyncState::Fail
        ) || subscription.sync.last_at.is_none()
    }) {
        return Some(unknown_reason::WATCHLIST_NOT_SYNCED);
    }
    subscriptions
        .iter()
        .any(|subscription| {
            subscription
                .sync
                .last_at
                .is_some_and(|last_at| now - last_at > watchlist_freshness_window(subscription))
        })
        .then_some(unknown_reason::WATCHLIST_STALE)
}

/// Who holds each title of the batch on their watchlist, or why that cannot
/// be said for any of them.
pub(crate) fn watchlists_from_rows(
    rows: &[ListMembership],
    subscriptions: &[ListSubscription],
    now: DateTime<Utc>,
) -> WatchlistLoad {
    let watchlists: HashMap<&str, &ListSubscription> = subscriptions
        .iter()
        .filter(|subscription| is_member_watchlist(subscription))
        .map(|subscription| (subscription.id.as_str(), subscription))
        .collect();
    let all: Vec<&ListSubscription> = watchlists.values().copied().collect();
    if let Some(reason) = watchlists_unavailable_reason(&all, now) {
        return WatchlistLoad::Unknown(reason);
    }
    let mut by_title: HashMap<String, TitleWatchlist> = HashMap::new();
    for row in rows.iter().filter(|row| row.left_at.is_none()) {
        let (Some(title_id), Some(subscription)) = (
            row.title_id.as_deref(),
            watchlists.get(row.subscription_id.as_str()),
        ) else {
            continue;
        };
        let entry = by_title.entry(title_id.to_string()).or_default();
        entry
            .owner_user_ids
            .insert(subscription.owner_user_id.clone());
        entry.added_at = Some(
            entry
                .added_at
                .map_or(row.first_seen_at, |at| at.min(row.first_seen_at)),
        );
        entry
            .origins
            .insert(subscription.source.provider.trim().to_ascii_lowercase());
    }
    WatchlistLoad::Loaded(by_title)
}

impl AppUseCase {
    /// One read each of the requests, memberships and subscriptions for a
    /// batch. A failed read leaves the facts it feeds unknown, so the rule is
    /// held rather than the pass failing for rules that never mention a list.
    #[cfg(test)]
    pub(crate) async fn maintenance_list_facts_or_unknown(
        &self,
        titles: &[Title],
    ) -> MaintenanceListFacts {
        self.maintenance_list_facts_loading(titles, MaintenanceFactLoads::default(), Utc::now())
            .await
    }

    /// As [`Self::maintenance_list_facts_or_unknown`], also reading the
    /// watchlists when `loads` asks for them. The subscriptions are the same
    /// one read either way.
    pub(crate) async fn maintenance_list_facts_loading(
        &self,
        titles: &[Title],
        loads: MaintenanceFactLoads,
        now: DateTime<Utc>,
    ) -> MaintenanceListFacts {
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        if title_ids.is_empty() {
            return MaintenanceListFacts {
                lists: Some(HashMap::new()),
                origins: Some(HashMap::new()),
                watchlists: WatchlistLoad::NotLoaded,
                metadata: MetadataLoad::default(),
                episode_shapes: EpisodeShapeLoad::default(),
            };
        }
        let metadata = if loads.metadata {
            self.maintenance_metadata_rows(&title_ids).await
        } else {
            MetadataLoad::default()
        };
        let episode_shapes = if loads.episode_shapes {
            self.maintenance_episode_shape_rows(titles, now.date_naive())
                .await
        } else {
            EpisodeShapeLoad::default()
        };
        let origins = match self
            .services
            .catalog
            .media_requests
            .request_origins_by_title_ids(&title_ids)
            .await
        {
            Ok(origins) => Some(origins),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read request origins; request origin facts are unknown for this batch"
                );
                None
            }
        };
        let needs_public_names = origins.as_ref().is_some_and(|origins| {
            origins
                .values()
                .any(|origin| matches!(origin, MediaRequestOrigin::PublicList { .. }))
        });
        let store = match self
            .maintenance_list_store_rows(&title_ids, needs_public_names || loads.watchlists)
            .await
        {
            Ok(store) => Some(store),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read list memberships; list facts are unknown for this batch"
                );
                None
            }
        };
        let watchlists = match (&store, loads.watchlists) {
            (_, false) => WatchlistLoad::NotLoaded,
            (None, true) => WatchlistLoad::Unknown(unknown_reason::LIST_STORE_UNREADABLE),
            (Some((rows, subscriptions)), true) => watchlists_from_rows(rows, subscriptions, now),
        };
        MaintenanceListFacts {
            watchlists,
            metadata,
            episode_shapes,
            origins: origins.map(|origins| {
                let subscriptions = store
                    .as_ref()
                    .map_or(&[][..], |(_, subscriptions)| subscriptions);
                request_origins_with_names(origins, subscriptions)
            }),
            lists: store.map(|(rows, subscriptions)| list_facts_from_rows(&rows, &subscriptions)),
        }
    }

    /// One grouped read of the batch's shows' episode lists. Movies are not
    /// read, and a batch with no show reads nothing.
    async fn maintenance_episode_shape_rows(
        &self,
        titles: &[Title],
        today: chrono::NaiveDate,
    ) -> EpisodeShapeLoad {
        let show_ids: Vec<String> = titles
            .iter()
            .filter(|title| !matches!(title.facet, MediaFacet::Movie))
            .map(|title| title.id.clone())
            .collect();
        let shapes = if show_ids.is_empty() {
            Some(HashMap::new())
        } else {
            match self
                .services
                .catalog
                .titles
                .list_title_episode_shapes(&show_ids, today)
                .await
            {
                Ok(shapes) => Some(shapes.into_iter().collect()),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "could not read episode shapes; episode-shape facts are unknown for this batch"
                    );
                    None
                }
            }
        };
        EpisodeShapeLoad {
            loaded: true,
            show_title_ids: show_ids.into_iter().collect(),
            shapes,
        }
    }

    /// One bounded read each of the batch's genres and ratings.
    async fn maintenance_metadata_rows(&self, title_ids: &[String]) -> MetadataLoad {
        let titles = &self.services.catalog.titles;
        let genres = match titles.list_title_genres(title_ids).await {
            Ok(genres) => Some(genres.into_iter().collect()),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read title genres; genre facts are unknown for this batch"
                );
                None
            }
        };
        let ratings = match titles.list_title_ratings(title_ids).await {
            Ok(ratings) => Some(ratings.into_iter().collect()),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read title ratings; rating facts are unknown for this batch"
                );
                None
            }
        };
        MetadataLoad {
            loaded: true,
            genres,
            ratings,
        }
    }

    /// The memberships of a batch, and the subscriptions when there are any
    /// memberships or a request needs a public list's name.
    async fn maintenance_list_store_rows(
        &self,
        title_ids: &[String],
        needs_subscriptions: bool,
    ) -> AppResult<(Vec<ListMembership>, Vec<ListSubscription>)> {
        let lists = &self.services.lists;
        let rows = lists.memberships.list_by_titles(title_ids).await?;
        if rows.is_empty() && !needs_subscriptions {
            return Ok((rows, Vec::new()));
        }
        let subscriptions = lists
            .subscriptions
            .list(ListSubscriptionQuery::default())
            .await?;
        Ok((rows, subscriptions))
    }
}

pub(crate) fn list_facts_from_rows(
    rows: &[ListMembership],
    subscriptions: &[ListSubscription],
) -> ListFactsByTitle {
    let subscriptions: HashMap<&str, &ListSubscription> = subscriptions
        .iter()
        .map(|subscription| (subscription.id.as_str(), subscription))
        .collect();
    let mut by_title: HashMap<&str, Vec<&ListMembership>> = HashMap::new();
    for row in rows {
        if let Some(title_id) = row.title_id.as_deref() {
            by_title.entry(title_id).or_default().push(row);
        }
    }

    by_title
        .into_iter()
        .map(|(title_id, rows)| {
            let present: Vec<&ListMembership> = rows
                .iter()
                .copied()
                .filter(|row| row.left_at.is_none())
                .collect();
            let mut names = BTreeSet::new();
            let mut enabled_ids = BTreeSet::new();
            let mut on_leave = BTreeSet::new();
            let mut public_ids = BTreeSet::new();
            let mut personal_ids = BTreeSet::new();
            for row in &present {
                let Some(subscription) = subscriptions.get(row.subscription_id.as_str()) else {
                    continue;
                };
                match subscription.scope {
                    ListScope::Public => public_ids.insert(subscription.id.as_str()),
                    ListScope::Personal => personal_ids.insert(subscription.id.as_str()),
                };
                if !subscription.enabled {
                    continue;
                }
                enabled_ids.insert(subscription.id.clone());
                on_leave.insert(subscription.on_leave.as_str().to_string());
                if subscription.scope == ListScope::Public {
                    names.insert(subscription.name.clone());
                }
            }
            let left_all = rows.iter().all(|row| row.left_at.is_some());
            let left_at = left_all
                .then(|| rows.iter().filter_map(|row| row.left_at).max())
                .flatten();
            let last_list_name = rows
                .iter()
                .filter_map(|row| {
                    let subscription = subscriptions.get(row.subscription_id.as_str())?;
                    (subscription.scope == ListScope::Public)
                        .then_some((row.left_at?, subscription.name.clone()))
                })
                .max_by(|left, right| left.0.cmp(&right.0).then(right.1.cmp(&left.1)))
                .map(|(_, name)| name);
            let departed: Vec<bool> = rows
                .iter()
                .filter(|row| row.left_at.is_some())
                .map(|row| row.left_handled)
                .collect();
            (
                title_id.to_string(),
                TitleListFacts {
                    added_by_list: rows.iter().any(|row| row.added_by_list),
                    on_enabled_list: !enabled_ids.is_empty(),
                    names: names.into_iter().collect(),
                    left_all,
                    left_at,
                    last_list_name,
                    enabled_ids: enabled_ids.into_iter().collect(),
                    public_count: public_ids.len() as i64,
                    personal_count: personal_ids.len() as i64,
                    first_seen_at: present.iter().map(|row| row.first_seen_at).min(),
                    last_seen_at: present.iter().map(|row| row.last_seen_at).max(),
                    best_rank: present.iter().filter_map(|row| row.rank).min(),
                    on_leave: on_leave.into_iter().collect(),
                    left_handled: (!departed.is_empty())
                        .then(|| departed.iter().all(|handled| *handled)),
                    membership_states: present
                        .iter()
                        .map(|row| row.state.as_str().to_string())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                },
            )
        })
        .collect()
}

/// Name each public-list origin from the subscriptions; every other origin,
/// and a public list the store no longer holds, stays unnamed.
pub(crate) fn request_origins_with_names(
    origins: HashMap<String, MediaRequestOrigin>,
    subscriptions: &[ListSubscription],
) -> HashMap<String, TitleRequestOrigin> {
    origins
        .into_iter()
        .map(|(title_id, origin)| {
            let list_name = match &origin {
                MediaRequestOrigin::PublicList { subscription_id } => subscriptions
                    .iter()
                    .find(|subscription| {
                        subscription.id == *subscription_id
                            && subscription.scope == ListScope::Public
                    })
                    .map(|subscription| subscription.name.clone()),
                _ => None,
            };
            (title_id, TitleRequestOrigin { origin, list_name })
        })
        .collect()
}

/// Lay the batch's list and request-origin facts over one subject's snapshot.
/// A store that could not be read leaves every fact it feeds unknown.
pub(crate) fn apply_list_facts(
    input: &mut MaintenanceInput,
    loaded: &MaintenanceListFacts,
    title_id: &str,
) {
    let evaluation_time = input.evaluation_time;
    let facts = &mut input.facts;
    let origin = loaded
        .origins
        .as_ref()
        .map(|origins| origins.get(title_id).cloned());
    (facts.request_origin, facts.request_list_name) = match origin {
        None => {
            let reason = unknown_reason::REQUEST_STORE_UNREADABLE;
            (Observation::unknown(reason), Observation::unknown(reason))
        }
        Some(None) => {
            let reason = unknown_reason::TITLE_NOT_REQUESTED;
            (
                Observation::absent_because(reason),
                Observation::absent_because(reason),
            )
        }
        Some(Some(TitleRequestOrigin { origin, list_name })) => (
            Observation::known(origin.as_str().to_string()),
            match (list_name, &origin, &loaded.lists) {
                (Some(name), _, _) => Observation::known(name),
                (None, MediaRequestOrigin::PublicList { .. }, None) => {
                    Observation::unknown(unknown_reason::LIST_STORE_UNREADABLE)
                }
                (None, _, _) => Observation::absent_because(unknown_reason::NO_PUBLIC_REQUEST_LIST),
            },
        ),
    };

    let Some(lists) = &loaded.lists else {
        let reason = unknown_reason::LIST_STORE_UNREADABLE;
        facts.lists_added_by_list = Observation::unknown(reason);
        facts.lists_on_enabled_list = Observation::unknown(reason);
        facts.lists_names = Observation::unknown(reason);
        facts.lists_left_all = Observation::unknown(reason);
        facts.lists_left_at = Observation::unknown(reason);
        facts.lists_last_list_name = Observation::unknown(reason);
        facts.lists_enabled_ids = Observation::unknown(reason);
        facts.lists_public_count = Observation::unknown(reason);
        facts.lists_personal_count = Observation::unknown(reason);
        facts.lists_first_seen_at = Observation::unknown(reason);
        facts.lists_last_seen_at = Observation::unknown(reason);
        facts.lists_days_on_list = Observation::unknown(reason);
        facts.lists_best_rank = Observation::unknown(reason);
        facts.lists_on_leave = Observation::unknown(reason);
        facts.lists_left_handled = Observation::unknown(reason);
        facts.lists_membership_states = Observation::unknown(reason);
        return;
    };
    let title = lists.get(title_id).cloned().unwrap_or_default();
    fn not_on_a_list<T: serde::Serialize>() -> Observation<T> {
        Observation::absent_because(unknown_reason::NOT_ON_A_LIST)
    }
    facts.lists_added_by_list = Observation::known(title.added_by_list);
    facts.lists_on_enabled_list = Observation::known(title.on_enabled_list);
    facts.lists_names = Observation::known(title.names);
    facts.lists_left_all = Observation::known(title.left_all);
    facts.lists_left_at = title.left_at.map_or_else(
        || Observation::absent_because(unknown_reason::NOT_LEFT_EVERY_LIST),
        |left_at| Observation::known(left_at.to_rfc3339()),
    );
    facts.lists_last_list_name = title.last_list_name.map_or_else(
        || Observation::absent_because(unknown_reason::NO_PUBLIC_LIST_LEFT),
        Observation::known,
    );
    facts.lists_enabled_ids = Observation::known(title.enabled_ids);
    facts.lists_public_count = Observation::known(title.public_count);
    facts.lists_personal_count = Observation::known(title.personal_count);
    facts.lists_first_seen_at = title
        .first_seen_at
        .map_or_else(not_on_a_list, |at| Observation::known(at.to_rfc3339()));
    facts.lists_last_seen_at = title
        .last_seen_at
        .map_or_else(not_on_a_list, |at| Observation::known(at.to_rfc3339()));
    facts.lists_days_on_list = title.first_seen_at.map_or_else(not_on_a_list, |at| {
        Observation::known((evaluation_time - at).num_days().max(0))
    });
    facts.lists_best_rank = title.best_rank.map_or_else(
        || Observation::absent_because(unknown_reason::NO_LIST_RANK),
        Observation::known,
    );
    facts.lists_on_leave = Observation::known(title.on_leave);
    facts.lists_left_handled = title.left_handled.map_or_else(
        || Observation::absent_because(unknown_reason::NO_LIST_LEFT),
        Observation::known,
    );
    facts.lists_membership_states = Observation::known(title.membership_states);
}

/// Lay the batch's watchlist, genre and rating facts over one subject's
/// snapshot. A group the batch did not load stays unknown.
pub(crate) fn apply_watchlist_and_metadata_facts(
    input: &mut MaintenanceInput,
    loaded: &MaintenanceListFacts,
    title_id: &str,
    people: MaintenanceTitlePeople<'_>,
) {
    apply_metadata_facts(input, &loaded.metadata, title_id);
    apply_episode_shape_facts(input, &loaded.episode_shapes, title_id);
    apply_watchlist_facts(input, loaded, title_id, people);
}

fn apply_episode_shape_facts(
    input: &mut MaintenanceInput,
    loaded: &EpisodeShapeLoad,
    title_id: &str,
) {
    // A movie keeps the absence the builder gave it.
    if !loaded.loaded || !loaded.show_title_ids.contains(title_id) {
        return;
    }
    let facts = &mut input.facts;
    let Some(shapes) = &loaded.shapes else {
        let reason = unknown_reason::EPISODE_STORE_UNREADABLE;
        facts.season_count = Observation::unknown(reason);
        facts.aired_episode_count = Observation::unknown(reason);
        facts.unaired_episode_count = Observation::unknown(reason);
        facts.missing_episode_count = Observation::unknown(reason);
        facts.aired_episode_file_percent = Observation::unknown(reason);
        facts.latest_season_number = Observation::unknown(reason);
        facts.latest_aired_episode_date = Observation::unknown(reason);
        facts.next_episode_air_date = Observation::unknown(reason);
        return;
    };
    let shape = shapes.get(title_id).cloned().unwrap_or_default();
    facts.season_count = Observation::known(shape.season_count);
    facts.aired_episode_count = Observation::known(shape.aired_episode_count);
    facts.unaired_episode_count = Observation::known(shape.unaired_episode_count);
    facts.missing_episode_count = Observation::known(shape.missing_episode_count);
    facts.aired_episode_file_percent = if shape.aired_episode_count > 0 {
        Observation::known(
            shape.aired_episode_file_count as f64 * 100.0 / shape.aired_episode_count as f64,
        )
    } else {
        Observation::absent_because(unknown_reason::NO_AIRED_EPISODE)
    };
    facts.latest_season_number = shape
        .latest_season_number
        .map_or_else(Observation::absent, Observation::known);
    facts.latest_aired_episode_date = shape.latest_aired_episode_date.map_or_else(
        || Observation::absent_because(unknown_reason::NO_AIRED_EPISODE),
        Observation::known,
    );
    facts.next_episode_air_date = shape.next_episode_air_date.map_or_else(
        || Observation::absent_because(unknown_reason::NO_SCHEDULED_EPISODE),
        Observation::known,
    );
}

fn apply_metadata_facts(input: &mut MaintenanceInput, loaded: &MetadataLoad, title_id: &str) {
    if !loaded.loaded {
        return;
    }
    let facts = &mut input.facts;
    facts.genres = loaded.genres.as_ref().map_or_else(
        || Observation::unknown(unknown_reason::METADATA_UNREADABLE),
        |genres| Observation::known(genres.get(title_id).cloned().unwrap_or_default()),
    );
    let Some(ratings) = &loaded.ratings else {
        facts.rating = Observation::unknown(unknown_reason::METADATA_UNREADABLE);
        facts.ratings = Observation::unknown(unknown_reason::METADATA_UNREADABLE);
        return;
    };
    let summary = ratings.get(title_id);
    facts.rating = summary
        .and_then(|summary| summary.rating)
        .filter(|rating| rating.is_finite())
        .map_or_else(Observation::absent, Observation::known);
    facts.ratings = Observation::known(
        summary
            .map(|summary| {
                summary
                    .external_ratings
                    .iter()
                    .map(|rating| MaintenanceRatingDoc {
                        source: rating.source.clone(),
                        value: rating.value.filter(|value| value.is_finite()),
                        normalized: rating.normalized,
                        votes: rating.votes.map(i64::from),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
}

fn apply_watchlist_facts(
    input: &mut MaintenanceInput,
    loaded: &MaintenanceListFacts,
    title_id: &str,
    people: MaintenanceTitlePeople<'_>,
) {
    let facts = &mut input.facts;
    let by_title = match &loaded.watchlists {
        WatchlistLoad::NotLoaded => return,
        WatchlistLoad::Unknown(reason) => {
            let reason = *reason;
            facts.watchlisted_by_user_ids = Observation::unknown(reason);
            facts.watchlisted_by_usernames = Observation::unknown(reason);
            facts.watchlisted_by_anyone = Observation::unknown(reason);
            facts.watchlisted_by_any_requester = Observation::unknown(reason);
            facts.watchlist_added_at = Observation::unknown(reason);
            facts.watchlist_origins = Observation::unknown(reason);
            return;
        }
        WatchlistLoad::Loaded(by_title) => by_title,
    };
    let title = by_title.get(title_id).cloned().unwrap_or_default();
    // All or nothing, as for the requester names: a shorter list would read
    // as "this person does not want it" for an account that no longer exists.
    facts.watchlisted_by_usernames = title
        .owner_user_ids
        .iter()
        .map(|user_id| people.usernames.get(user_id).cloned())
        .collect::<Option<Vec<String>>>()
        .map_or_else(
            || Observation::unknown(unknown_reason::USER_NOT_FOUND),
            Observation::known,
        );
    let requesters = people.requester_user_ids.unwrap_or_default();
    facts.watchlisted_by_any_requester = if requesters.is_empty() {
        Observation::absent_because(unknown_reason::TITLE_NOT_REQUESTED)
    } else {
        Observation::known(
            requesters
                .iter()
                .any(|requester| title.owner_user_ids.contains(requester)),
        )
    };
    facts.watchlisted_by_anyone = Observation::known(!title.owner_user_ids.is_empty());
    facts.watchlist_added_at = title.added_at.map_or_else(
        || Observation::absent_because(unknown_reason::NOT_ON_A_WATCHLIST),
        |at| Observation::known(at.to_rfc3339()),
    );
    facts.watchlist_origins = Observation::known(title.origins.into_iter().collect());
    facts.watchlisted_by_user_ids = Observation::known(title.owner_user_ids.into_iter().collect());
}

#[cfg(test)]
#[path = "list_facts_tests.rs"]
mod tests;
