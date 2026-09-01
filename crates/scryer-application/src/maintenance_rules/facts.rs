//! Fact snapshots for maintenance rule evaluation (RFC 137 section 8).
//!
//! Every fact the matcher can read is built here, from ports Scryer already
//! has. A fact Scryer cannot resolve becomes [`Observation::unknown`] with a
//! stable code rather than a plausible-looking default: an unknown fact holds
//! the rule (`unknown` beats `match` in the decoder), which is the fail-closed
//! behaviour the RFC demands. Guessing `false`, `0`, or "the newest file" would
//! silently authorize a destructive action on evidence Scryer never had.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use scryer_domain::{MediaFacet, Title};
use scryer_rules::maintenance::{
    MAINTENANCE_INPUT_SCHEMA_VERSION, MaintenanceFactsDoc, MaintenanceFileDoc, MaintenanceInput,
    MaintenanceLibraryDoc, MaintenanceSubjectDoc, MaintenanceSubjectKind, Observation,
};

use crate::types::TitleMediaFile;

/// Structured tag prefix carrying a title's quality profile. Kept in sync with
/// [`crate::ports::TitleRepository::count_by_quality_profile_id`], which
/// defines the trim-after-strip resolver semantics reused here.
pub(crate) const QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";

/// Stable reason codes for facts this wave cannot observe.
///
/// These are part of the rule-authoring contract: a matcher that tests
/// `input.facts.<fact>.reason` compares against exactly these strings.
pub mod unknown_reason {
    /// Scryer does not record this signal yet. Applies to `last_upgraded_at`
    /// (no distinct upgrade event exists — the newest file timestamp is a
    /// different fact and must not stand in for it), `active_downloads`
    /// (download-state wiring is out of scope for the foundation wave), and the
    /// three episode counts for series-facet titles (only per-title episode
    /// queries exist, so a preview over 50 titles would fan out).
    pub const NOT_YET_COLLECTED: &str = "not_yet_collected";

    /// The title has files, but none of their rows carry a parseable
    /// timestamp, so the first-import instant is genuinely unavailable.
    pub const FILE_TIMESTAMPS_UNAVAILABLE: &str = "file_timestamps_unavailable";

    /// The title carries no `created_by`, which is what a scan- or
    /// discovery-created title looks like: nobody added it through the add
    /// flow. This is an *absence* reason, not an unknown one — the source
    /// answered, and the answer is that there is no adding user.
    pub const TITLE_ADDED_BY_SYSTEM: &str = "title_added_by_system";

    /// A user id Scryer holds no longer resolves to a user row. The id itself
    /// stays known; only the name is unavailable, so a rule matching on names
    /// is held rather than told the wrong one.
    pub const USER_NOT_FOUND: &str = "user_not_found";
}

/// Everything the builder needs about one library, resolved once per run.
#[derive(Clone, Debug)]
pub struct MaintenanceLibraryRef {
    pub id: String,
    pub name: String,
}

/// The people signals for one title, resolved by the caller in batch.
///
/// Both fields are prefetched once per evaluation run, exactly like the media
/// files: resolving them per title would make the job's cost scale with the
/// library.
#[derive(Clone, Copy, Debug)]
pub struct MaintenanceTitlePeople<'a> {
    /// Every user linked to this title through a media request — the original
    /// submitter plus any additional requesters — deduped, in a stable order.
    /// `None` means no media request created this title, which is a confirmed
    /// answer and becomes `requested = false`, not an unknown.
    pub requester_user_ids: Option<&'a [String]>,
    /// Username by user id for every user that existed when the run started.
    /// A missing id is a user that no longer exists, not a lookup that failed.
    pub usernames: &'a HashMap<String, String>,
}

/// Build the title-scoped input document for one title.
///
/// `files` must be this title's files only; callers batch-load for the whole
/// selection and group by `title_id` rather than querying per title. The same
/// holds for [`MaintenanceTitlePeople`].
pub fn build_title_input(
    evaluation_time: DateTime<Utc>,
    title: &Title,
    library: &MaintenanceLibraryRef,
    files: &[TitleMediaFile],
    people: MaintenanceTitlePeople<'_>,
) -> MaintenanceInput {
    MaintenanceInput {
        schema_version: MAINTENANCE_INPUT_SCHEMA_VERSION,
        evaluation_time,
        subject: MaintenanceSubjectDoc {
            kind: MaintenanceSubjectKind::Title,
            title_id: title.id.clone(),
            season_number: None,
            episode_id: None,
            facet: title.facet.as_str().to_string(),
            name: title.name.clone(),
            year: title.year,
        },
        library: MaintenanceLibraryDoc {
            id: library.id.clone(),
            name: library.name.clone(),
        },
        facts: build_facts(title, files, people),
    }
}

fn build_facts(
    title: &Title,
    files: &[TitleMediaFile],
    people: MaintenanceTitlePeople<'_>,
) -> MaintenanceFactsDoc {
    let file_docs: Vec<MaintenanceFileDoc> = files.iter().map(file_doc).collect();
    let total_size: i64 = files.iter().map(|file| file.size_bytes).sum();
    let (added_by_user_id, added_by_username) =
        added_by_observations(title.created_by.as_deref(), people.usernames);
    let (requested, requested_by_user_ids, requested_by_usernames) =
        requested_observations(people.requester_user_ids, people.usernames);

    MaintenanceFactsDoc {
        monitored: Observation::known(title.monitored),
        tags: Observation::known(title.tags.clone()),
        quality_profile_id: quality_profile_observation(&title.tags),
        added_at: Observation::known(title.created_at.to_rfc3339()),
        added_by_user_id,
        added_by_username,
        requested,
        requested_by_user_ids,
        requested_by_usernames,
        first_imported_at: first_imported_observation(files),
        // Scryer records no distinct upgrade event. Approximating it with the
        // newest file timestamp would make "upgraded" and "imported" the same
        // fact, so rules asking about upgrades are held instead.
        last_upgraded_at: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
        // An empty file set is a confirmed answer, not a missing one.
        has_file: Observation::known(!files.is_empty()),
        file_count: Observation::known(files.len() as i64),
        total_file_size_bytes: Observation::known(total_size),
        files: Observation::known(file_docs),
        episode_count: episode_count_observation(&title.facet),
        episode_file_count: episode_count_observation(&title.facet),
        monitored_episode_count: episode_count_observation(&title.facet),
        active_downloads: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
    }
}

/// Known when the structured tag is present and non-empty; absent when the
/// title carries no such tag, which is a confirmed "no profile assigned"
/// rather than a lookup Scryer failed to perform.
fn quality_profile_observation(tags: &[String]) -> Observation<String> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(QUALITY_PROFILE_TAG_PREFIX))
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
        .map_or_else(Observation::absent, |profile_id| {
            Observation::known(profile_id.to_string())
        })
}

/// Who added the title, as an id and as a name.
///
/// No `created_by` is a confirmed answer, not a gap: the title arrived from a
/// scan or from discovery rather than through the add flow, so both facts are
/// absent and carry [`unknown_reason::TITLE_ADDED_BY_SYSTEM`]. An id that no
/// longer resolves is the opposite case — the id is still known, but the name
/// is genuinely unavailable, so the name is unknown rather than falling back
/// to the raw id, which a rule comparing usernames would read as a real name.
fn added_by_observations(
    created_by: Option<&str>,
    usernames: &HashMap<String, String>,
) -> (Observation<String>, Observation<String>) {
    let Some(user_id) = created_by.map(str::trim).filter(|id| !id.is_empty()) else {
        return (
            Observation::absent_because(unknown_reason::TITLE_ADDED_BY_SYSTEM),
            Observation::absent_because(unknown_reason::TITLE_ADDED_BY_SYSTEM),
        );
    };

    let username = usernames.get(user_id).map_or_else(
        || Observation::unknown(unknown_reason::USER_NOT_FOUND),
        |username| Observation::known(username.clone()),
    );
    (Observation::known(user_id.to_string()), username)
}

/// Whether a media request created this title, and who asked for it.
///
/// `requested` is never absent: "no media request is linked to this title" is
/// something Scryer looked up and confirmed, so it is a known `false`, and the
/// two lists are then known-empty rather than absent.
///
/// Username resolution is deliberately all-or-nothing: if any requester id
/// fails to resolve, `requested_by_usernames` is unknown instead of a shorter
/// list. A partial list is worse than no list, because the rule cannot tell it
/// apart from a complete one — `not "alice" in input.facts.requested_by_usernames.value`
/// would silently hold on a run where alice's user row is the one that is
/// missing. The ids stay known in that case, so a rule that wants to proceed
/// can match on those instead.
fn requested_observations(
    requester_user_ids: Option<&[String]>,
    usernames: &HashMap<String, String>,
) -> (
    Observation<bool>,
    Observation<Vec<String>>,
    Observation<Vec<String>>,
) {
    let Some(user_ids) = requester_user_ids else {
        return (
            Observation::known(false),
            Observation::known(Vec::new()),
            Observation::known(Vec::new()),
        );
    };

    let resolved: Option<Vec<String>> = user_ids
        .iter()
        .map(|user_id| usernames.get(user_id).cloned())
        .collect();

    (
        Observation::known(true),
        Observation::known(user_ids.to_vec()),
        resolved.map_or_else(
            || Observation::unknown(unknown_reason::USER_NOT_FOUND),
            Observation::known,
        ),
    )
}

/// Earliest file timestamp. Absent when the title has no files at all; unknown
/// only when files exist but carry no readable timestamp.
fn first_imported_observation(files: &[TitleMediaFile]) -> Observation<String> {
    if files.is_empty() {
        return Observation::absent();
    }

    files
        .iter()
        .filter_map(|file| parse_timestamp(&file.created_at))
        .min()
        .map_or_else(
            || Observation::unknown(unknown_reason::FILE_TIMESTAMPS_UNAVAILABLE),
            |earliest| Observation::known(earliest.to_rfc3339()),
        )
}

/// Absent for movies — a movie has no episodes, and that is a fact, not a gap.
/// Unknown for series and anime until an episode-count port exists that a
/// batched preview can afford.
fn episode_count_observation(facet: &MediaFacet) -> Observation<i64> {
    match facet {
        MediaFacet::Movie => Observation::absent(),
        MediaFacet::Series | MediaFacet::Anime => {
            Observation::unknown(unknown_reason::NOT_YET_COLLECTED)
        }
    }
}

/// Fields the media-file row does not carry stay `None`; the file document
/// declares them optional, so a rule reading one gets undefined rather than a
/// fabricated value.
fn file_doc(file: &TitleMediaFile) -> MaintenanceFileDoc {
    MaintenanceFileDoc {
        size_bytes: Some(file.size_bytes),
        quality: file.quality_label.clone(),
        video_codec: file
            .video_codec
            .or(file.video_codec_parsed)
            .map(|codec| codec.as_str().to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        audio_languages: file.audio_languages.clone(),
        subtitle_languages: file.subtitle_languages.clone(),
        added_at: parse_timestamp(&file.created_at).map(|added| added.to_rfc3339()),
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(created_by: Option<&str>) -> Title {
        let facet = MediaFacet::Movie;
        Title {
            id: "title-1".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            name: "Provenance Fixture".to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/Movies"),
            created_by: created_by.map(str::to_string),
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn usernames(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(id, username)| ((*id).to_string(), (*username).to_string()))
            .collect()
    }

    fn library() -> MaintenanceLibraryRef {
        MaintenanceLibraryRef {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
        }
    }

    fn facts(
        title: &Title,
        requester_user_ids: Option<&[String]>,
        usernames: &HashMap<String, String>,
    ) -> serde_json::Value {
        let input = build_title_input(
            Utc::now(),
            title,
            &library(),
            &[],
            MaintenanceTitlePeople {
                requester_user_ids,
                usernames,
            },
        );
        serde_json::to_value(input.facts).expect("facts serialize")
    }

    #[test]
    fn a_title_added_through_the_add_flow_reports_who_added_it() {
        let names = usernames(&[("user-1", "operator-one")]);
        let facts = facts(&title(Some("user-1")), None, &names);

        assert_eq!(facts["added_by_user_id"]["status"], "known");
        assert_eq!(facts["added_by_user_id"]["value"], "user-1");
        assert_eq!(facts["added_by_username"]["status"], "known");
        assert_eq!(facts["added_by_username"]["value"], "operator-one");
    }

    #[test]
    fn a_scan_created_title_is_absent_with_the_system_reason_not_unknown() {
        let facts = facts(&title(None), None, &usernames(&[]));

        // Absent, not unknown: Scryer looked and confirmed nobody added it, so
        // a rule asking "was this added by a person" gets a decisive answer
        // rather than being held.
        for fact in ["added_by_user_id", "added_by_username"] {
            assert_eq!(facts[fact]["status"], "absent", "{fact}");
            assert_eq!(facts[fact]["reason"], "title_added_by_system", "{fact}");
            assert!(facts[fact].get("value").is_none(), "{fact}");
        }
    }

    #[test]
    fn a_blank_created_by_reads_as_system_added() {
        let facts = facts(&title(Some("   ")), None, &usernames(&[]));

        assert_eq!(facts["added_by_user_id"]["status"], "absent");
        assert_eq!(facts["added_by_user_id"]["reason"], "title_added_by_system");
    }

    #[test]
    fn an_adding_user_that_no_longer_exists_leaves_the_name_unknown() {
        let facts = facts(&title(Some("user-gone")), None, &usernames(&[]));

        // The id is still a fact Scryer holds; only the name is unavailable.
        assert_eq!(facts["added_by_user_id"]["status"], "known");
        assert_eq!(facts["added_by_user_id"]["value"], "user-gone");
        assert_eq!(facts["added_by_username"]["status"], "unknown");
        assert_eq!(facts["added_by_username"]["reason"], "user_not_found");
    }

    #[test]
    fn a_title_no_request_created_is_a_known_false_with_empty_lists() {
        let facts = facts(&title(Some("user-1")), None, &usernames(&[]));

        assert_eq!(facts["requested"]["status"], "known");
        assert_eq!(facts["requested"]["value"], false);
        assert_eq!(facts["requested_by_user_ids"]["status"], "known");
        assert_eq!(
            facts["requested_by_user_ids"]["value"],
            serde_json::json!([])
        );
        assert_eq!(facts["requested_by_usernames"]["status"], "known");
        assert_eq!(
            facts["requested_by_usernames"]["value"],
            serde_json::json!([])
        );
    }

    #[test]
    fn a_requested_title_carries_every_requester_in_the_order_it_was_given() {
        let names = usernames(&[("user-1", "operator-one"), ("user-2", "viewer-two")]);
        let ids = vec!["user-1".to_string(), "user-2".to_string()];
        let facts = facts(&title(Some("user-1")), Some(&ids), &names);

        assert_eq!(facts["requested"]["value"], true);
        assert_eq!(
            facts["requested_by_user_ids"]["value"],
            serde_json::json!(["user-1", "user-2"])
        );
        assert_eq!(
            facts["requested_by_usernames"]["value"],
            serde_json::json!(["operator-one", "viewer-two"])
        );
    }

    #[test]
    fn an_empty_requester_list_still_reports_the_title_as_requested() {
        // Key presence is the answer, not list length: a linked request with no
        // resolvable requester rows is still a request.
        let facts = facts(&title(None), Some(&[]), &usernames(&[]));

        assert_eq!(facts["requested"]["value"], true);
        assert_eq!(
            facts["requested_by_user_ids"]["value"],
            serde_json::json!([])
        );
        assert_eq!(
            facts["requested_by_usernames"]["value"],
            serde_json::json!([])
        );
    }

    #[test]
    fn one_unresolvable_requester_makes_the_whole_username_list_unknown() {
        let names = usernames(&[("user-1", "operator-one")]);
        let ids = vec!["user-1".to_string(), "user-gone".to_string()];
        let facts = facts(&title(Some("user-1")), Some(&ids), &names);

        // A partial list is indistinguishable from a complete one, so it is
        // withheld entirely; the ids stay known for rules that can use them.
        assert_eq!(facts["requested_by_usernames"]["status"], "unknown");
        assert_eq!(facts["requested_by_usernames"]["reason"], "user_not_found");
        assert!(facts["requested_by_usernames"].get("value").is_none());
        assert_eq!(
            facts["requested_by_user_ids"]["value"],
            serde_json::json!(["user-1", "user-gone"])
        );
    }
}
