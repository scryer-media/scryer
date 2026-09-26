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

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use scryer_domain::{ListMembership, ListScope, ListSubscription, Title};
use scryer_rules::maintenance::{MaintenanceFactsDoc, Observation};

use super::facts::unknown_reason;
use crate::lists::ListSubscriptionQuery;
use crate::{AppResult, AppUseCase};

/// What the list store says about one title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleListFacts {
    pub added_by_list: bool,
    pub on_enabled_list: bool,
    pub names: Vec<String>,
    pub left_all: bool,
    pub left_at: Option<DateTime<Utc>>,
    pub last_list_name: Option<String>,
}

/// List facts for a batch, keyed by title id. A title with no entry is on no
/// list and never was.
pub(crate) type ListFactsByTitle = HashMap<String, TitleListFacts>;

impl AppUseCase {
    /// One read of the subscriptions and one of the memberships for a batch.
    pub(crate) async fn maintenance_list_facts_for_titles(
        &self,
        titles: &[Title],
    ) -> AppResult<ListFactsByTitle> {
        let title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let lists = &self.services.lists;
        let rows = lists.memberships.list_by_titles(&title_ids).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let subscriptions = lists
            .subscriptions
            .list(ListSubscriptionQuery::default())
            .await?;
        Ok(list_facts_from_rows(&rows, &subscriptions))
    }

    /// [`Self::maintenance_list_facts_for_titles`], with a failed read turned
    /// into `None` so the facts read unknown and the rule is held rather than
    /// the pass failing for rules that never mention a list.
    pub(crate) async fn maintenance_list_facts_or_unknown(
        &self,
        titles: &[Title],
    ) -> Option<ListFactsByTitle> {
        match self.maintenance_list_facts_for_titles(titles).await {
            Ok(facts) => Some(facts),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read list memberships; list facts are unknown for this batch"
                );
                None
            }
        }
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
            let present = rows.iter().filter(|row| row.left_at.is_none());
            let mut on_enabled_list = false;
            let mut names = BTreeSet::new();
            for row in present {
                let Some(subscription) = subscriptions.get(row.subscription_id.as_str()) else {
                    continue;
                };
                if !subscription.enabled {
                    continue;
                }
                on_enabled_list = true;
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
            (
                title_id.to_string(),
                TitleListFacts {
                    added_by_list: rows.iter().any(|row| row.added_by_list),
                    on_enabled_list,
                    names: names.into_iter().collect(),
                    left_all,
                    left_at,
                    last_list_name,
                },
            )
        })
        .collect()
}

/// Lay the batch's list facts over one subject's snapshot. `None` means the
/// list store could not be read, and every list fact reads unknown.
pub(crate) fn apply_list_facts(
    facts: &mut MaintenanceFactsDoc,
    loaded: Option<&ListFactsByTitle>,
    title_id: &str,
) {
    let Some(loaded) = loaded else {
        let reason = unknown_reason::LIST_STORE_UNREADABLE;
        facts.lists_added_by_list = Observation::unknown(reason);
        facts.lists_on_enabled_list = Observation::unknown(reason);
        facts.lists_names = Observation::unknown(reason);
        facts.lists_left_all = Observation::unknown(reason);
        facts.lists_left_at = Observation::unknown(reason);
        facts.lists_last_list_name = Observation::unknown(reason);
        return;
    };
    let title = loaded.get(title_id).cloned().unwrap_or_default();
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
}

#[cfg(test)]
#[path = "list_facts_tests.rs"]
mod tests;
