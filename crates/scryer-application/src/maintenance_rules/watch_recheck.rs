//! The live watch re-check an action makes before it acts on a watch fact.
//!
//! A rule that reads a watch fact decided on what the last sweep stored, which
//! can be hours old. Before such an action runs, every item the stored facts
//! name for the subject is read from its server again, for every account the
//! gate counts on that connection, and on Plex the server's own history for
//! the item is read too. Anything the stored facts do not account for holds
//! the action; a clean re-check only lets it proceed. Nothing read here is
//! stored: the next sweep remains the one writer of watch facts.
//!
//! A subject nobody had played at the sweep has no stored rows to name its
//! items, so its items are found through the media-server library mapping
//! (the playback mapping each connection keeps from title and episode to the
//! server's item) and read the same way. A subject with files that some
//! observed connection has no mapped item for cannot be re-checked there, and
//! holds with its own reason, apart from a read that failed.
//!
//! Times are compared to the whole second. The servers report finer than the
//! stores keep (.NET ticks are 100 ns; Postgres keeps microseconds), so an
//! exact compare would read every round-tripped time as a newer play.

use std::collections::{BTreeMap, HashMap, HashSet};

use scryer_domain::{
    MaintenanceRuleSubjectKind, MediaFacet, MediaServerPlaybackEntityKind, UserMediaSignal,
};

use super::action_execution::execution_reason;
use super::facts::{MaintenanceWatchContext, WatchSignalFreshness};
use crate::AppUseCase;
use crate::ports::{AudiencePlay, ProviderItemRecheck};

/// What a live re-check found, as the hold it causes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WatchRecheck {
    /// Every item read back as the stored facts describe it.
    Clear,
    /// A counted account played an item since the stored facts were read.
    NewerPlay,
    /// The server's history names a play the stored facts credit to no one.
    UncreditedPlay,
    /// An item or account could not be read back.
    Unavailable,
    /// An observed connection has no mapped item for the subject, so no
    /// read was attempted there.
    Unmapped,
}

impl WatchRecheck {
    /// The `hold_reason` this outcome writes, or `None` for a clean read.
    pub(crate) fn hold_reason(self) -> Option<&'static str> {
        match self {
            Self::Clear => None,
            Self::NewerPlay => Some(execution_reason::WATCH_RECHECK_NEWER_PLAY),
            Self::UncreditedPlay => Some(execution_reason::WATCH_RECHECK_UNCREDITED_PLAY),
            Self::Unavailable => Some(execution_reason::WATCH_RECHECK_UNAVAILABLE),
            Self::Unmapped => Some(execution_reason::WATCH_RECHECK_UNMAPPED),
        }
    }
}

/// Whether a matcher reads any watch fact. A source whose facts cannot be
/// listed counts as reading one, so it is re-checked rather than waved through.
pub(crate) fn rule_reads_watch_facts(rego_source: &str, rule_set_id: &str) -> bool {
    match scryer_rules::validation::maintenance_referenced_facts(rego_source, rule_set_id) {
        Ok(facts) => scryer_rules::maintenance::WATCH_MAINTENANCE_FACTS
            .iter()
            .any(|fact| facts.contains(*fact)),
        Err(_) => true,
    }
}

/// A play time to the whole second, the precision every store keeps.
fn whole_seconds(at: Option<chrono::DateTime<chrono::Utc>>) -> Option<i64> {
    at.map(|at| at.timestamp())
}

/// The catalog entities whose server items a re-check must read: the movie,
/// or each of the subject's episodes one of its files is linked to. `None`
/// when a file cannot be placed on an entity the mapping knows (a series file
/// the episode inventory does not link to any episode), so the subject's items
/// cannot all be found.
///
/// A subject with no files has nothing on any server to play, so nothing
/// needs finding; its stored rows are still re-checked.
pub(crate) fn recheck_entities(
    facet: &MediaFacet,
    title_id: &str,
    subject: &super::subjects::MaintenanceSubject,
) -> Option<Vec<(MediaServerPlaybackEntityKind, String)>> {
    if subject.files.is_empty() {
        return Some(Vec::new());
    }
    if *facet == MediaFacet::Movie && subject.kind == MaintenanceRuleSubjectKind::Title {
        return Some(vec![(
            MediaServerPlaybackEntityKind::Title,
            title_id.to_string(),
        )]);
    }
    // Episode links come from the inventory: a file row read for a season or
    // episode scope carries no episode id of its own.
    let inventory = subject.inventory.as_ref()?;
    let in_subject: HashSet<&str> = subject
        .episodes
        .iter()
        .map(|episode| episode.id.as_str())
        .collect();
    let mut entities = Vec::new();
    for file in &subject.files {
        let linked = inventory.file_episode_ids(&file.id)?;
        if linked.is_empty() {
            return None;
        }
        for episode_id in linked.iter().filter(|id| in_subject.contains(id.as_str())) {
            let entity = (MediaServerPlaybackEntityKind::Episode, episode_id.clone());
            if !entities.contains(&entity) {
                entities.push(entity);
            }
        }
    }
    Some(entities)
}

/// Compare one item's live state with what is stored for it on its connection.
///
/// `stored` holds this item's rows on this connection, one per account that
/// had played it at the sweep. `audience` holds the item's audience rows, one
/// per account of any kind; a history play one of them already covers is
/// credited. `participants` are the accounts that were asked about; each must
/// be in the answer.
pub(crate) fn fold_item_recheck(
    stored: &[&UserMediaSignal],
    audience: &[&AudiencePlay],
    participants: &[String],
    recheck: &ProviderItemRecheck,
) -> WatchRecheck {
    let stored_for = |account: &str| {
        stored
            .iter()
            .find(|signal| signal.external_user_id == account)
            .copied()
    };
    for participant in participants {
        let Some(live) = recheck
            .participants
            .iter()
            .find(|state| state.external_user_id == *participant)
        else {
            return WatchRecheck::Unavailable;
        };
        if !live.played {
            // Unplayed now. If the sweep stored a play, the stored facts say
            // more than the server; that is the next sweep's to correct, and
            // never a reason to act sooner.
            continue;
        }
        match stored_for(participant).filter(|signal| signal.played) {
            None => return WatchRecheck::NewerPlay,
            Some(signal)
                if whole_seconds(live.last_played_at) > whole_seconds(signal.last_played_at) =>
            {
                return WatchRecheck::NewerPlay;
            }
            Some(_) => {}
        }
    }
    let covers = |stored_at: Option<chrono::DateTime<chrono::Utc>>,
                  played_at: Option<chrono::DateTime<chrono::Utc>>| {
        match (stored_at, played_at) {
            (Some(stored_at), Some(played_at)) => stored_at.timestamp() >= played_at.timestamp(),
            (_, None) => true,
            (None, Some(_)) => false,
        }
    };
    for play in &recheck.history {
        let credited = stored_for(&play.external_user_id).is_some_and(|signal| {
            signal.played && covers(signal.last_played_at, play.last_played_at)
        }) || audience.iter().any(|row| {
            row.external_account_id == play.external_user_id
                && covers(row.last_played_at, play.last_played_at)
        });
        if credited {
            continue;
        }
        return if participants.contains(&play.external_user_id) {
            WatchRecheck::NewerPlay
        } else {
            WatchRecheck::UncreditedPlay
        };
    }
    WatchRecheck::Clear
}

impl AppUseCase {
    /// Read the subject's items back from their servers.
    ///
    /// `signals` are the title's stored rows and `audience` its audience
    /// rows; `episode_ids`, when given, narrows both to a season or episode
    /// subject. Every item either names is read, and so is every item the
    /// library mapping holds for `entities` (see [`recheck_entities`]; `None`
    /// is a subject whose items cannot all be found). Each connection's items
    /// are read together, for every account the gate counts there.
    ///
    /// Any read that cannot be made, any item with no id to read it by, and
    /// a gate that is not open are [`WatchRecheck::Unavailable`]. An entity
    /// an observed connection has no mapped item for is
    /// [`WatchRecheck::Unmapped`], decided before any read.
    pub(crate) async fn maintenance_watch_recheck(
        &self,
        context: &MaintenanceWatchContext,
        signals: &[UserMediaSignal],
        audience: &[AudiencePlay],
        episode_ids: Option<&HashSet<String>>,
        entities: Option<&[(MediaServerPlaybackEntityKind, String)]>,
    ) -> WatchRecheck {
        // The anonymous facts can be open while the per-user ones are not
        // (no linked user yet); either open gate is a picture to re-check.
        if context.freshness != WatchSignalFreshness::Fresh
            && context.audience_unavailable_reason().is_some()
        {
            return WatchRecheck::Unavailable;
        }
        let Some(entities) = entities else {
            return WatchRecheck::Unavailable;
        };
        let in_scope = |episode_id: Option<&String>| {
            episode_ids.is_none_or(|ids| episode_id.is_some_and(|id| ids.contains(id)))
        };
        let relevant: Vec<&UserMediaSignal> = signals
            .iter()
            .filter(|signal| {
                context
                    .enabled_connection_ids
                    .contains(&signal.connection_id)
            })
            .filter(|signal| in_scope(signal.scryer_episode_id.as_ref()))
            .collect();
        let relevant_audience: Vec<&AudiencePlay> = audience
            .iter()
            .filter(|play| context.enabled_connection_ids.contains(&play.connection_id))
            .filter(|play| in_scope(play.scryer_episode_id.as_ref()))
            .collect();
        if relevant
            .iter()
            .any(|signal| signal.provider_item_id.trim().is_empty())
        {
            return WatchRecheck::Unavailable;
        }

        // The subject's items on every observed connection, from the library
        // mapping: a title nobody played at the sweep has no stored row to
        // name them.
        let mapped = if entities.is_empty() {
            Vec::new()
        } else {
            match self
                .services
                .integrations
                .media_server_connections
                .list_playback_items_for_entities(entities)
                .await
            {
                Ok(mapped) => mapped,
                Err(_) => return WatchRecheck::Unavailable,
            }
        };
        let resolved = context.enabled_connection_ids.iter().all(|connection_id| {
            entities.iter().all(|(kind, id)| {
                mapped.iter().any(|item| {
                    item.connection_id == *connection_id
                        && item.entity_kind == *kind
                        && item.entity_id == *id
                        && !item.provider_item_id.trim().is_empty()
                })
            })
        });
        if !resolved {
            return WatchRecheck::Unmapped;
        }

        let roster = match self
            .services
            .integrations
            .media_server_signals
            .signal_roster()
            .await
        {
            Ok(roster) => roster,
            Err(_) => return WatchRecheck::Unavailable,
        };
        let mut participants_by_connection: HashMap<&str, Vec<String>> = HashMap::new();
        for entry in &roster {
            if context.linked_user_ids.contains(&entry.link.scryer_user_id) {
                let accounts = participants_by_connection
                    .entry(entry.connection_id.as_str())
                    .or_default();
                if !accounts.contains(&entry.external_user_id) {
                    accounts.push(entry.external_user_id.clone());
                }
            }
        }

        // Connection → item → that item's stored and audience rows.
        type ItemRows<'a> = (Vec<&'a UserMediaSignal>, Vec<&'a AudiencePlay>);
        let mut items: BTreeMap<&str, BTreeMap<&str, ItemRows<'_>>> = BTreeMap::new();
        for signal in relevant {
            items
                .entry(signal.connection_id.as_str())
                .or_default()
                .entry(signal.provider_item_id.as_str())
                .or_default()
                .0
                .push(signal);
        }
        for play in relevant_audience {
            items
                .entry(play.connection_id.as_str())
                .or_default()
                .entry(play.provider_item_id.as_str())
                .or_default()
                .1
                .push(play);
        }
        for item in &mapped {
            if context.enabled_connection_ids.contains(&item.connection_id) {
                items
                    .entry(item.connection_id.as_str())
                    .or_default()
                    .entry(item.provider_item_id.trim())
                    .or_default();
            }
        }

        for (connection_id, connection_items) in items {
            let connection = match self
                .services
                .integrations
                .media_server_connections
                .get_by_id(connection_id)
                .await
            {
                Ok(Some(connection)) if connection.enabled => connection,
                _ => return WatchRecheck::Unavailable,
            };
            let reader = match self
                .services
                .integrations
                .media_server_signal_source
                .open(&connection)
                .await
            {
                Ok(reader) => reader,
                Err(_) => return WatchRecheck::Unavailable,
            };
            let participants = participants_by_connection
                .get(connection_id)
                .cloned()
                .unwrap_or_default();
            let item_ids: Vec<String> = connection_items.keys().map(|id| id.to_string()).collect();
            let answers = match reader.recheck_items(&item_ids, &participants).await {
                Ok(answers) => answers,
                Err(_) => return WatchRecheck::Unavailable,
            };
            for (item_id, (stored, audience)) in connection_items {
                let Some(recheck) = answers.get(item_id) else {
                    return WatchRecheck::Unavailable;
                };
                let outcome = fold_item_recheck(&stored, &audience, &participants, recheck);
                if outcome != WatchRecheck::Clear {
                    return outcome;
                }
            }
        }
        WatchRecheck::Clear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ProviderItemPlay;
    use chrono::{DateTime, Duration, Utc};
    use scryer_domain::{MediaServerProvider, MediaServerSignalKind};

    fn at(hours_ago: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_900_000_000, 0).expect("fixed instant")
            - Duration::hours(hours_ago)
    }

    fn stored(account: &str, last_played_at: Option<DateTime<Utc>>) -> UserMediaSignal {
        UserMediaSignal {
            id: format!("signal-{account}"),
            connection_id: "conn-example".to_string(),
            provider: MediaServerProvider::Plex,
            external_user_id: account.to_string(),
            scryer_user_id: Some(format!("user-{account}")),
            provider_item_id: "item-1".to_string(),
            kind: MediaServerSignalKind::Movie,
            scryer_title_id: Some("title-1".to_string()),
            scryer_episode_id: None,
            played: true,
            play_count: 1,
            last_played_at,
            is_favorite: false,
            observed_at: at(0),
            sync_generation: 1,
            created_at: at(0),
            updated_at: at(0),
        }
    }

    fn live(
        account: &str,
        played: bool,
        last_played_at: Option<DateTime<Utc>>,
    ) -> ProviderItemPlay {
        ProviderItemPlay {
            external_user_id: account.to_string(),
            played,
            last_played_at,
        }
    }

    fn accounts(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_live_state_matching_the_stored_facts_is_clear() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(at(48))), live("200", false, None)],
            history: vec![live("100", true, Some(at(48)))],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100", "200"]), &recheck),
            WatchRecheck::Clear
        );
    }

    /// Jellyfin and Emby report seven fractional digits (.NET ticks); the
    /// store keeps microseconds. The same play read back must not look newer.
    #[test]
    fn a_play_read_back_finer_than_the_store_keeps_is_the_same_play() {
        let server_time = DateTime::parse_from_rfc3339("2026-10-30T08:15:42.1234567Z")
            .expect("seven fractional digits parse")
            .with_timezone(&Utc);
        let stored_time = DateTime::parse_from_rfc3339("2026-10-30T08:15:42.123456Z")
            .expect("microseconds parse")
            .with_timezone(&Utc);
        assert!(
            server_time > stored_time,
            "the fixture must differ below a second"
        );
        let row = stored("100", Some(stored_time));
        let audience_row = AudiencePlay {
            connection_id: "conn-example".to_string(),
            external_account_id: "900".to_string(),
            provider_item_id: "item-1".to_string(),
            kind: MediaServerSignalKind::Movie,
            scryer_title_id: Some("title-1".to_string()),
            scryer_episode_id: None,
            play_count: 1,
            last_played_at: Some(stored_time),
        };
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(server_time))],
            history: vec![
                live("100", true, Some(server_time)),
                live("900", true, Some(server_time)),
            ],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[&audience_row], &accounts(&["100"]), &recheck),
            WatchRecheck::Clear
        );

        // A play one second later is a newer play.
        let later = ProviderItemRecheck {
            participants: vec![live("100", true, Some(server_time + Duration::seconds(1)))],
            history: Vec::new(),
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100"]), &later),
            WatchRecheck::NewerPlay
        );
    }

    #[test]
    fn a_first_play_since_the_sweep_is_a_newer_play() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![
                live("100", true, Some(at(48))),
                live("200", true, Some(at(1))),
            ],
            history: Vec::new(),
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100", "200"]), &recheck),
            WatchRecheck::NewerPlay
        );
    }

    #[test]
    fn a_rewatch_since_the_sweep_is_a_newer_play() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(at(1)))],
            history: Vec::new(),
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100"]), &recheck),
            WatchRecheck::NewerPlay
        );
    }

    #[test]
    fn an_item_unplayed_since_the_sweep_does_not_hold() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", false, None)],
            history: Vec::new(),
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100"]), &recheck),
            WatchRecheck::Clear
        );
    }

    #[test]
    fn a_participant_missing_from_the_answer_is_unavailable() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(at(48)))],
            history: Vec::new(),
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100", "200"]), &recheck),
            WatchRecheck::Unavailable
        );
    }

    #[test]
    fn a_history_play_by_an_account_no_fact_credits_is_uncredited() {
        let row = stored("100", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(at(48)))],
            history: vec![
                live("100", true, Some(at(48))),
                live("900", true, Some(at(200))),
            ],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100"]), &recheck),
            WatchRecheck::UncreditedPlay
        );
    }

    #[test]
    fn a_history_play_newer_than_a_counted_accounts_stored_play_is_a_newer_play() {
        // A user read from history: their own state comes from the same rows,
        // and a row past the stored time is a play the sweep did not see.
        let row = stored("300", Some(at(48)));
        let recheck = ProviderItemRecheck {
            participants: vec![live("300", true, Some(at(48)))],
            history: vec![
                live("300", true, Some(at(2))),
                live("300", true, Some(at(48))),
            ],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["300"]), &recheck),
            WatchRecheck::NewerPlay
        );
    }

    #[test]
    fn a_stored_play_without_a_time_does_not_credit_a_timed_history_play() {
        let row = stored("100", None);
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, None)],
            history: vec![live("100", true, Some(at(5)))],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[], &accounts(&["100"]), &recheck),
            WatchRecheck::NewerPlay
        );
    }

    #[test]
    fn a_history_play_an_audience_row_covers_is_credited() {
        let row = stored("100", Some(at(48)));
        let audience_row = AudiencePlay {
            connection_id: "conn-example".to_string(),
            external_account_id: "900".to_string(),
            provider_item_id: "item-1".to_string(),
            kind: MediaServerSignalKind::Movie,
            scryer_title_id: Some("title-1".to_string()),
            scryer_episode_id: None,
            play_count: 1,
            last_played_at: Some(at(200)),
        };
        let recheck = ProviderItemRecheck {
            participants: vec![live("100", true, Some(at(48)))],
            history: vec![
                live("100", true, Some(at(48))),
                live("900", true, Some(at(200))),
            ],
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[&audience_row], &accounts(&["100"]), &recheck),
            WatchRecheck::Clear
        );
        // A play by that account after the audience read is still uncredited.
        let newer = ProviderItemRecheck {
            history: vec![live("900", true, Some(at(3)))],
            ..recheck
        };
        assert_eq!(
            fold_item_recheck(&[&row], &[&audience_row], &accounts(&["100"]), &newer),
            WatchRecheck::UncreditedPlay
        );
    }

    #[test]
    fn every_hold_names_a_distinct_execution_reason() {
        let reasons = [
            WatchRecheck::NewerPlay,
            WatchRecheck::UncreditedPlay,
            WatchRecheck::Unavailable,
        ]
        .map(|outcome| outcome.hold_reason().expect("a hold has a reason"));
        let distinct = reasons.iter().collect::<HashSet<_>>();
        assert_eq!(distinct.len(), reasons.len());
        assert_eq!(WatchRecheck::Clear.hold_reason(), None);
    }

    #[test]
    fn a_rule_reads_watch_facts_only_through_a_watch_fact() {
        let watch = "package whatever\nimport rego.v1\n\nmatch if {\n\tcount(input.facts.watched_by_user_ids) == 0\n}\n";
        let media = "package whatever\nimport rego.v1\n\nmatch if {\n\tinput.facts.monitored\n}\n";
        assert!(rule_reads_watch_facts(watch, "rule-a"));
        assert!(!rule_reads_watch_facts(media, "rule-a"));
    }
}
