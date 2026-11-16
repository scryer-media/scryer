//! The sources the web editor ships as templates, pinned here so a server
//! change that would refuse one fails a test instead of an operator's save.

use scryer_rules::lifecycle::rewrite_package_declaration;
use scryer_rules::validation::validate_lifecycle_rule;

use super::prepare_matcher;

const WEB_TEMPLATES: &[(&str, &str)] = &[
    (
        "dead-wanted-entries",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.monitored == true\n\tinput.facts.has_file == false\n}\n\nunmonitor if rule_applies\n",
    ),
    (
        "library-aging",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.has_file == true\n\tis_array(input.facts.tags)\n\tnot \"keep\" in input.facts.tags\n}\n\ndelete_title if rule_applies\n",
    ),
    (
        "added-over-180-days-ago",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tdays_since(input.facts.added_at) > 180\n}\n\nunmonitor if rule_applies\n",
    ),
    (
        "oversized-releases",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tis_number(input.facts.total_file_size_bytes)\n\tinput.facts.total_file_size_bytes > 40000000000\n}\n\nset_quality_profile := \"Compact 1080p\" if rule_applies\n\nsearch if rule_applies\n",
    ),
    (
        "four-k-purge",
        "package rules\nimport rego.v1\n\nholds_2160p if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := \"Compact 1080p\" if holds_2160p\n\nsearch if holds_2160p\n",
    ),
    (
        "requested-media-expiry",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.requested == true\n\tis_array(input.facts.tags)\n\tnot \"keep\" in input.facts.tags\n}\n\ndelete_title if rule_applies\n",
    ),
    (
        "departed-requester",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\t\"departed-user\" in input.facts.requested_by_usernames\n}\n\ndelete_title if rule_applies\n",
    ),
    (
        "system-added-cleanup",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tnot input.facts.added_by_user_id != null\n\tinput.facts.has_file == true\n}\n\nunmonitor if rule_applies\n",
    ),
    (
        "watched-by-every-requester",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.requested == true\n\tinput.facts.watched_by_all_requesters == true\n}\n\ndelete_title if rule_applies\n",
    ),
    (
        "expired-request-leases",
        "package rules\nimport rego.v1\n\ndelete_title if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n",
    ),
    (
        "tagged-for-removal",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\t\"remove\" in input.facts.tags\n}\n\ndelete_title if rule_applies\n",
    ),
    (
        "flag-for-review",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.has_file == true\n\tdays_since(input.facts.first_imported_at) > 365\n}\n\nadd_tags := [\"needs-review\"] if rule_applies\n",
    ),
    (
        "no-quality-profile",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tnot input.facts.quality_profile_id != null\n}\n\nadd_tags := [\"needs-review\"] if rule_applies\n",
    ),
    (
        "keep-newest-three-downloaded-episodes",
        "package rules\nimport rego.v1\n\nrule_applies_1 if {\n\tis_number(input.episode.facts.episode_position_by_air_date)\n\tinput.episode.facts.episode_position_by_air_date > 3\n}\n\ndelete_episode_files if rule_applies_1\n",
    ),
    (
        "keep-newest-two-downloaded-seasons",
        "package rules\nimport rego.v1\n\nrule_applies_1 if {\n\tis_number(input.season.facts.season_position_by_air_date)\n\tinput.season.facts.season_position_by_air_date > 2\n}\n\ndelete_season_files if rule_applies_1\n",
    ),
    (
        "left-every-public-list",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.lists_left_all == true\n\tinput.facts.lists_left_handled == true\n\tdays_since(input.facts.lists_left_at) > 30\n}\n\nunmonitor if rule_applies\n",
    ),
    (
        "list-added-never-watched",
        "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.lists_added_by_list == true\n\tis_number(input.facts.lists_days_on_list)\n\tinput.facts.lists_days_on_list > 90\n\tcount(input.facts.watched_by_user_ids) == 0\n}\n\nadd_tags := [\"needs-review\"] if rule_applies\n",
    ),
];

#[test]
fn every_web_template_saves_without_errors_or_warnings() {
    assert_eq!(WEB_TEMPLATES.len(), 17);
    for (template, source) in WEB_TEMPLATES {
        let rule_id = format!("template_{}", template.replace('-', "_"));
        let rewritten = rewrite_package_declaration(source, &rule_id);
        let validation = validate_lifecycle_rule(&rewritten, &rule_id)
            .unwrap_or_else(|error| panic!("{template}: {error}"));
        assert!(
            validation.valid && validation.diagnostics.is_empty(),
            "{template}: {:?}",
            validation.diagnostics
        );
        assert!(
            validation.warnings.is_empty(),
            "{template}: {:?}",
            validation.warnings
        );
        let prepared = prepare_matcher(&rule_id, source, 7)
            .unwrap_or_else(|error| panic!("{template}: {error:?}"));
        assert!(!prepared.declared.heads.is_empty(), "{template}");
    }
}
