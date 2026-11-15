import type { MaintenanceRuleScope } from "@/lib/types/maintenance-rule-sets";

/// Which kind of title a template was written for. The maintenance editor scopes
/// a rule by library rather than by facet, so this is descriptive metadata the
/// gallery shows rather than a field the draft carries: it tells the operator
/// what the matcher assumes before they pick libraries for it.
export type MaintenanceTemplateFacet = "movie" | "show";

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
export type MaintenanceRuleTemplate = {
  id: string;
  /// Prefilled rule name. Deliberately not derived from the localized title:
  /// the name is an operator-facing identifier that should not change shape
  /// when the UI language does.
  name: string;
  titleKey: string;
  descriptionKey: string;
  /// The scope the example needs. Omitted examples remain title-scoped.
  subjectKind?: MaintenanceRuleScope;
  /// True when the source names a placeholder quality profile the operator
  /// has to replace with one of their own before the rule is useful.
  requiresTargetQualityProfile?: boolean;
  graceDays: number;
  subjectFacets: MaintenanceTemplateFacet[];
  /// True when the source declares `delete_files` or `delete_title`. The
  /// gallery marks these, and their copy says so, even though the instance
  /// gates and per-rule state still have to agree before anything is removed.
  destructive?: boolean;
  /// The most titles one run acts on. Set exactly on the destructive
  /// templates, since a rule that deletes cannot be saved without it.
  maxTitlesPerRun?: number;
  regoSource: string;
};

export const MAINTENANCE_RULE_TEMPLATES: MaintenanceRuleTemplate[] = [
  {
    id: "dead-wanted-entries",
    name: "dead_wanted_entries",
    titleKey: "settings.maintenanceTemplateDeadWantedTitle",
    descriptionKey: "settings.maintenanceTemplateDeadWantedDescription",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nwanted_without_files if {\n\tinput.facts.monitored\n\tnot input.facts.has_file\n}\n\nunmonitor if wanted_without_files\n",
  },
  {
    id: "library-aging",
    name: "library_aging",
    titleKey: "settings.maintenanceTemplateLibraryAgingTitle",
    descriptionKey: "settings.maintenanceTemplateLibraryAgingDescription",
    graceDays: 180,
    subjectFacets: ["movie"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\naging_unkept if {\n\tinput.facts.has_file\n\tnot "keep" in input.facts.tags\n}\n\ndelete_title if aging_unkept\n',
  },
  {
    id: "added-over-180-days-ago",
    name: "added_over_180_days_ago",
    titleKey: "settings.maintenanceTemplateAddedLongAgoTitle",
    descriptionKey: "settings.maintenanceTemplateAddedLongAgoDescription",
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      "package rules\nimport rego.v1\n\nunmonitor if days_since(input.facts.added_at) > 180\n",
  },
  {
    id: "oversized-releases",
    name: "oversized_releases",
    titleKey: "settings.maintenanceTemplateOversizedTitle",
    descriptionKey: "settings.maintenanceTemplateOversizedDescription",
    requiresTargetQualityProfile: true,
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      'package rules\nimport rego.v1\n\noversized if input.facts.total_file_size_bytes > 40000000000\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := "Compact 1080p" if oversized\n\nsearch if oversized\n',
  },
  {
    id: "four-k-purge",
    name: "four_k_purge",
    titleKey: "settings.maintenanceTemplateFourKPurgeTitle",
    descriptionKey: "settings.maintenanceTemplateFourKPurgeDescription",
    requiresTargetQualityProfile: true,
    graceDays: 7,
    subjectFacets: ["movie"],
    regoSource:
      'package rules\nimport rego.v1\n\nholds_2160p if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n\n# Replace with the name of the quality profile to move these titles to.\nset_quality_profile := "Compact 1080p" if holds_2160p\n\nsearch if holds_2160p\n',
  },
  {
    id: "requested-media-expiry",
    name: "requested_media_expiry",
    titleKey: "settings.maintenanceTemplateRequestedExpiryTitle",
    descriptionKey: "settings.maintenanceTemplateRequestedExpiryDescription",
    graceDays: 120,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\nrequested_unkept if {\n\tinput.facts.requested\n\tnot "keep" in input.facts.tags\n}\n\ndelete_title if requested_unkept\n',
  },
  {
    id: "departed-requester",
    name: "departed_requester",
    titleKey: "settings.maintenanceTemplateDepartedRequesterTitle",
    descriptionKey: "settings.maintenanceTemplateDepartedRequesterDescription",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\ndelete_title if {\n\t"departed-user" in input.facts.requested_by_usernames\n}\n',
  },
  {
    id: "system-added-cleanup",
    name: "system_added_cleanup",
    titleKey: "settings.maintenanceTemplateSystemAddedTitle",
    descriptionKey: "settings.maintenanceTemplateSystemAddedDescription",
    graceDays: 60,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nsystem_added_with_files if {\n\tnot input.facts.added_by_user_id\n\tinput.facts.has_file\n}\n\nunmonitor if system_added_with_files\n",
  },
  {
    id: "watched-by-every-requester",
    name: "watched_by_every_requester",
    titleKey: "settings.maintenanceTemplateWatchedRequestedTitle",
    descriptionKey: "settings.maintenanceTemplateWatchedRequestedDescription",
    graceDays: 30,
    subjectFacets: ["movie"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\nwatched_by_requesters if {\n\tinput.facts.requested\n\tinput.facts.watched_by_all_requesters\n}\n\ndelete_title if watched_by_requesters\n",
  },
  {
    id: "expired-request-leases",
    name: "expired_request_leases",
    titleKey: "settings.maintenanceTemplateExpiredLeasesTitle",
    descriptionKey: "settings.maintenanceTemplateExpiredLeasesDescription",
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
    titleKey: "settings.maintenanceTemplateTaggedForRemovalTitle",
    descriptionKey: "settings.maintenanceTemplateTaggedForRemovalDescription",
    graceDays: 7,
    subjectFacets: ["movie", "show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      'package rules\nimport rego.v1\n\ndelete_title if {\n\t"remove" in input.facts.tags\n}\n',
  },
  {
    id: "flag-for-review",
    name: "flag_for_review",
    titleKey: "settings.maintenanceTemplateFlagForReviewTitle",
    descriptionKey: "settings.maintenanceTemplateFlagForReviewDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nimported_over_a_year_ago if {\n\tinput.facts.has_file\n\tdays_since(input.facts.first_imported_at) > 365\n}\n\nadd_tags := ["needs-review"] if imported_over_a_year_ago\n',
  },
  {
    id: "no-quality-profile",
    name: "no_quality_profile",
    titleKey: "settings.maintenanceTemplateNoProfileTitle",
    descriptionKey: "settings.maintenanceTemplateNoProfileDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nadd_tags := ["needs-review"] if not input.facts.quality_profile_id\n',
  },
  {
    id: "keep-newest-three-downloaded-episodes",
    name: "keep_newest_three_downloaded_episodes",
    titleKey: "settings.maintenanceTemplateNewestEpisodesTitle",
    descriptionKey: "settings.maintenanceTemplateNewestEpisodesDescription",
    subjectKind: "EPISODE",
    graceDays: 7,
    subjectFacets: ["show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\ndelete_files if input.facts.episode_position_by_air_date > 3\n",
  },
  {
    id: "keep-newest-two-downloaded-seasons",
    name: "keep_newest_two_downloaded_seasons",
    titleKey: "settings.maintenanceTemplateNewestSeasonsTitle",
    descriptionKey: "settings.maintenanceTemplateNewestSeasonsDescription",
    subjectKind: "SEASON",
    graceDays: 7,
    subjectFacets: ["show"],
    destructive: true,
    maxTitlesPerRun: 10,
    regoSource:
      "package rules\nimport rego.v1\n\ndelete_files if input.facts.season_position_by_air_date > 2\n",
  },
  {
    id: "left-every-public-list",
    name: "left_every_public_list",
    titleKey: "settings.maintenanceTemplateLeftEveryListTitle",
    descriptionKey: "settings.maintenanceTemplateLeftEveryListDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nleft_lists_a_month_ago if {\n\tinput.facts.lists_left_all\n\tinput.facts.lists_left_handled\n\tdays_since(input.facts.lists_left_at) > 30\n}\n\nunmonitor if left_lists_a_month_ago\n",
  },
  {
    id: "list-added-never-watched",
    name: "list_added_never_watched",
    titleKey: "settings.maintenanceTemplateListAddedUnwatchedTitle",
    descriptionKey: "settings.maintenanceTemplateListAddedUnwatchedDescription",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      'package rules\nimport rego.v1\n\nlist_added_unwatched if {\n\tinput.facts.lists_added_by_list\n\tinput.facts.lists_days_on_list > 90\n\tcount(input.facts.watched_by_user_ids) == 0\n}\n\nadd_tags := ["needs-review"] if list_added_unwatched\n',
  },
];

const FACET_LABEL_KEYS: Record<MaintenanceTemplateFacet, string> = {
  movie: "settings.maintenanceTemplateFacetMovie",
  show: "settings.maintenanceTemplateFacetShow",
};

export function maintenanceTemplateFacetLabelKey(
  facet: MaintenanceTemplateFacet,
): string {
  return FACET_LABEL_KEYS[facet];
}
