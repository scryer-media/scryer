//! The sources the web editor ships as templates, pinned here so a server
//! change that would refuse one fails a test instead of an operator's save.

use scryer_domain::LifecycleRuleSubjectKind;
use scryer_rules::lifecycle::rewrite_package_declaration;
use scryer_rules::validation::validate_lifecycle_rule;

use super::prepare_matcher;

const WEB_TEMPLATES: &[(&str, LifecycleRuleSubjectKind, &str)] = &[
    (
        "dead-wanted-entries",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nwanted_without_files if {\n\tinput.facts.monitored\n\tnot input.facts.has_file\n}\n\nunmonitor if wanted_without_files\n",
    ),
    (
        "library-aging",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\naging_unkept if {\n\tinput.facts.has_file\n\tnot \"keep\" in input.facts.tags\n}\n\ndelete_title if aging_unkept\n",
    ),
    (
        "added-over-180-days-ago",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nunmonitor if days_since(input.facts.added_at) > 180\n",
    ),
    (
        "oversized-releases",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\noversized if input.facts.total_file_size_bytes > 40000000000\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := \"Compact 1080p\" if oversized\n\nsearch if oversized\n",
    ),
    (
        "four-k-purge",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nholds_2160p if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := \"Compact 1080p\" if holds_2160p\n\nsearch if holds_2160p\n",
    ),
    (
        "requested-media-expiry",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nrequested_unkept if {\n\tinput.facts.requested\n\tnot \"keep\" in input.facts.tags\n}\n\ndelete_title if requested_unkept\n",
    ),
    (
        "departed-requester",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\ndelete_title if {\n\t\"departed-user\" in input.facts.requested_by_usernames\n}\n",
    ),
    (
        "system-added-cleanup",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nsystem_added_with_files if {\n\tnot input.facts.added_by_user_id\n\tinput.facts.has_file\n}\n\nunmonitor if system_added_with_files\n",
    ),
    (
        "watched-by-every-requester",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nwatched_by_requesters if {\n\tinput.facts.requested\n\tinput.facts.watched_by_all_requesters\n}\n\ndelete_title if watched_by_requesters\n",
    ),
    (
        "expired-request-leases",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\ndelete_title if {\n\tinput.facts.request_lease_state == \"expired\"\n}\n",
    ),
    (
        "tagged-for-removal",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\ndelete_title if {\n\t\"remove\" in input.facts.tags\n}\n",
    ),
    (
        "flag-for-review",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nimported_over_a_year_ago if {\n\tinput.facts.has_file\n\tdays_since(input.facts.first_imported_at) > 365\n}\n\nadd_tags := [\"needs-review\"] if imported_over_a_year_ago\n",
    ),
    (
        "no-quality-profile",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nadd_tags := [\"needs-review\"] if not input.facts.quality_profile_id\n",
    ),
    (
        "keep-newest-three-downloaded-episodes",
        LifecycleRuleSubjectKind::Episode,
        "package rules\nimport rego.v1\n\ndelete_files if input.facts.episode_position_by_air_date > 3\n",
    ),
    (
        "keep-newest-two-downloaded-seasons",
        LifecycleRuleSubjectKind::Season,
        "package rules\nimport rego.v1\n\ndelete_files if input.facts.season_position_by_air_date > 2\n",
    ),
    (
        "left-every-public-list",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nleft_lists_a_month_ago if {\n\tinput.facts.lists_left_all\n\tinput.facts.lists_left_handled\n\tdays_since(input.facts.lists_left_at) > 30\n}\n\nunmonitor if left_lists_a_month_ago\n",
    ),
    (
        "list-added-never-watched",
        LifecycleRuleSubjectKind::Title,
        "package rules\nimport rego.v1\n\nlist_added_unwatched if {\n\tinput.facts.lists_added_by_list\n\tinput.facts.lists_days_on_list > 90\n\tcount(input.facts.watched_by_user_ids) == 0\n}\n\nadd_tags := [\"needs-review\"] if list_added_unwatched\n",
    ),
];

#[test]
fn every_web_template_saves_without_errors_or_warnings() {
    assert_eq!(WEB_TEMPLATES.len(), 17);
    for (template, scope, source) in WEB_TEMPLATES {
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
        let prepared = prepare_matcher(&rule_id, *scope, source, 7, false)
            .unwrap_or_else(|error| panic!("{template}: {error:?}"));
        assert!(!prepared.declared.heads.is_empty(), "{template}");
    }
}
