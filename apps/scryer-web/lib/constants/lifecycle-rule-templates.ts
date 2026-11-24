/// Which kind of title a template was written for. The lifecycle editor scopes
/// a rule by library rather than by facet, so this is descriptive metadata the
/// gallery shows rather than a field the draft carries: it tells the operator
/// what the matcher assumes before they pick libraries for it.
export type LifecycleTemplateFacet = "movie" | "show";

/// A starter rule the operator can load into the create-rule editor. Nothing
/// here is ever saved on its own: applying a template prefills the draft and the
/// operator still reviews, names, scopes, and saves it themselves.
///
/// `regoSource` declares both what matches and what happens to it. It is pinned
/// byte-for-byte (tabs, trailing newline) against the fixtures the API
/// validates, so it is written with explicit escapes rather than a template
/// literal that a reformat could quietly re-indent. Tag labels and profile
/// names in a template are examples; the registry and the operator's own
/// profiles decide whether they exist.
export type LifecycleRuleTemplate = {
  id: string;
  /// Prefilled rule name. Deliberately not derived from the localized title:
  /// the name is an operator-facing identifier that should not change shape
  /// when the UI language does.
  name: string;
  titleKey: string;
  descriptionKey: string;
  /// True when the source names a placeholder quality profile the operator
  /// has to replace with one of their own before the rule is useful.
  requiresTargetQualityProfile?: boolean;
  graceDays: number;
  subjectFacets: LifecycleTemplateFacet[];
  /// True when the source declares a delete head at any level. The
  /// gallery marks these, and their copy says so, even though the instance
  /// gates and per-rule state still have to agree before anything is removed.
  destructive?: boolean;
  /// The most titles one run acts on. Set exactly on the destructive
  /// templates, since a rule that deletes cannot be saved without it.
  maxTitlesPerRun?: number;
  regoSource: string;
};

export const LIFECYCLE_RULE_TEMPLATES: LifecycleRuleTemplate[] = [
  {
    id: "dead-wanted-entries",
    name: "dead_wanted_entries",
    titleKey: "settings.lifecycleTemplateDeadWantedTitle",
    descriptionKey: "settings.lifecycleTemplateDeadWantedDescription",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.monitored == true\n\tinput.facts.has_file == false\n}\n\nunmonitor if rule_applies\n",
  },
  {
    id: "library-aging",
    name: "library_aging",
    titleKey: "settings.lifecycleTemplateLibraryAgingTitle",
    descriptionKey: "settings.lifecycleTemplateLibraryAgingDescription",
    graceDays: 180,
    subjectFacets: ["movie"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.has_file == true\n\tis_array(input.facts.tags)\n\tnot "keep" in input.facts.tags\n}\n\ndelete_title if rule_applies\n',
  },
  {
    id: "added-over-180-days-ago",
    name: "added_over_180_days_ago",
    titleKey: "settings.lifecycleTemplateAddedLongAgoTitle",
    descriptionKey: "settings.lifecycleTemplateAddedLongAgoDescription",
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies if {\n\tdays_since(input.facts.added_at) > 180\n}\n\nunmonitor if rule_applies\n",
  },
  {
    id: "oversized-releases",
    name: "oversized_releases",
    titleKey: "settings.lifecycleTemplateOversizedTitle",
    descriptionKey: "settings.lifecycleTemplateOversizedDescription",
    requiresTargetQualityProfile: true,
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tis_number(input.facts.total_file_size_bytes)\n\tinput.facts.total_file_size_bytes > 40000000000\n}\n\nset_quality_profile := "Compact 1080p" if rule_applies\n\nsearch if rule_applies\n',
  },
  {
    id: "four-k-purge",
    name: "four_k_purge",
    titleKey: "settings.lifecycleTemplateFourKPurgeTitle",
    descriptionKey: "settings.lifecycleTemplateFourKPurgeDescription",
    requiresTargetQualityProfile: true,
    graceDays: 7,
    subjectFacets: ["movie"],
    regoSource:
      'package rules\nimport rego.v1\n\nholds_2160p if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := "Compact 1080p" if holds_2160p\n\nsearch if holds_2160p\n',
  },
  {
    id: "requested-media-expiry",
    name: "requested_media_expiry",
    titleKey: "settings.lifecycleTemplateRequestedExpiryTitle",
    descriptionKey: "settings.lifecycleTemplateRequestedExpiryDescription",
    graceDays: 120,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.requested == true\n\tis_array(input.facts.tags)\n\tnot "keep" in input.facts.tags\n}\n\ndelete_title if rule_applies\n',
  },
  {
    id: "departed-requester",
    name: "departed_requester",
    titleKey: "settings.lifecycleTemplateDepartedRequesterTitle",
    descriptionKey: "settings.lifecycleTemplateDepartedRequesterDescription",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\t"departed-user" in input.facts.requested_by_usernames\n}\n\ndelete_title if rule_applies\n',
  },
  {
    id: "system-added-cleanup",
    name: "system_added_cleanup",
    titleKey: "settings.lifecycleTemplateSystemAddedTitle",
    descriptionKey: "settings.lifecycleTemplateSystemAddedDescription",
    graceDays: 60,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies if {\n\tnot input.facts.added_by_user_id != null\n\tinput.facts.has_file == true\n}\n\nunmonitor if rule_applies\n",
  },
  {
    id: "watched-by-every-requester",
    name: "watched_by_every_requester",
    titleKey: "settings.lifecycleTemplateWatchedRequestedTitle",
    descriptionKey: "settings.lifecycleTemplateWatchedRequestedDescription",
    graceDays: 30,
    subjectFacets: ["movie"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.requested == true\n\tinput.facts.watched_by_all_requesters == true\n}\n\ndelete_title if rule_applies\n",
  },
  {
    id: "expired-request-leases",
    name: "expired_request_leases",
    titleKey: "settings.lifecycleTemplateExpiredLeasesTitle",
    descriptionKey: "settings.lifecycleTemplateExpiredLeasesDescription",
    graceDays: 7,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\ndelete_title if {\n\tinput.facts.request_lease_state == "expired"\n}\n',
  },
  {
    id: "tagged-for-removal",
    name: "tagged_for_removal",
    titleKey: "settings.lifecycleTemplateTaggedForRemovalTitle",
    descriptionKey: "settings.lifecycleTemplateTaggedForRemovalDescription",
    graceDays: 7,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\t"remove" in input.facts.tags\n}\n\ndelete_title if rule_applies\n',
  },
  {
    id: "flag-for-review",
    name: "flag_for_review",
    titleKey: "settings.lifecycleTemplateFlagForReviewTitle",
    descriptionKey: "settings.lifecycleTemplateFlagForReviewDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.has_file == true\n\tdays_since(input.facts.first_imported_at) > 365\n}\n\nadd_tags := ["needs-review"] if rule_applies\n',
  },
  {
    id: "no-quality-profile",
    name: "no_quality_profile",
    titleKey: "settings.lifecycleTemplateNoProfileTitle",
    descriptionKey: "settings.lifecycleTemplateNoProfileDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tnot input.facts.quality_profile_id != null\n}\n\nadd_tags := ["needs-review"] if rule_applies\n',
  },
  {
    id: "keep-newest-three-downloaded-episodes",
    name: "keep_newest_three_downloaded_episodes",
    titleKey: "settings.lifecycleTemplateNewestEpisodesTitle",
    descriptionKey: "settings.lifecycleTemplateNewestEpisodesDescription",
    graceDays: 7,
    subjectFacets: ["show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies_1 if {\n\tis_number(input.episode.facts.episode_position_by_air_date)\n\tinput.episode.facts.episode_position_by_air_date > 3\n}\n\ndelete_episode_files if rule_applies_1\n",
  },
  {
    id: "keep-newest-two-downloaded-seasons",
    name: "keep_newest_two_downloaded_seasons",
    titleKey: "settings.lifecycleTemplateNewestSeasonsTitle",
    descriptionKey: "settings.lifecycleTemplateNewestSeasonsDescription",
    graceDays: 7,
    subjectFacets: ["show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies_1 if {\n\tis_number(input.season.facts.season_position_by_air_date)\n\tinput.season.facts.season_position_by_air_date > 2\n}\n\ndelete_season_files if rule_applies_1\n",
  },
  {
    id: "left-every-public-list",
    name: "left_every_public_list",
    titleKey: "settings.lifecycleTemplateLeftEveryListTitle",
    descriptionKey: "settings.lifecycleTemplateLeftEveryListDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.lists_left_all == true\n\tinput.facts.lists_left_handled == true\n\tdays_since(input.facts.lists_left_at) > 30\n}\n\nunmonitor if rule_applies\n",
  },
  {
    id: "list-added-never-watched",
    name: "list_added_never_watched",
    titleKey: "settings.lifecycleTemplateListAddedUnwatchedTitle",
    descriptionKey: "settings.lifecycleTemplateListAddedUnwatchedDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nrule_applies if {\n\tinput.facts.lists_added_by_list == true\n\tis_number(input.facts.lists_days_on_list)\n\tinput.facts.lists_days_on_list > 90\n\tcount(input.facts.watched_by_user_ids) == 0\n}\n\nadd_tags := ["needs-review"] if rule_applies\n',
  },
];

const FACET_LABEL_KEYS: Record<LifecycleTemplateFacet, string> = {
  movie: "settings.lifecycleTemplateFacetMovie",
  show: "settings.lifecycleTemplateFacetShow",
};

export function lifecycleTemplateFacetLabelKey(
  facet: LifecycleTemplateFacet,
): string {
  return FACET_LABEL_KEYS[facet];
}
