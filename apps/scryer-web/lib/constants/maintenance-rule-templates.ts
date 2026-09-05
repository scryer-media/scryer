import type { MaintenanceActionKind } from "@/lib/types/maintenance-rule-sets";

/// Which kind of title a template was written for. The maintenance editor scopes
/// a rule by library rather than by facet, so this is descriptive metadata the
/// gallery shows rather than a field the draft carries: it tells the operator
/// what the matcher assumes before they pick libraries for it.
export type MaintenanceTemplateFacet = "movie" | "show";

/// A starter rule the operator can load into the create-rule editor. Nothing
/// here is ever saved on its own: applying a template prefills the draft and the
/// operator still reviews, names, scopes, and saves it themselves.
///
/// `regoSource` is pinned byte-for-byte (tabs, trailing newline) against the
/// matcher fixtures the API validates, so it is written with explicit escapes
/// rather than a template literal that a reformat could quietly re-indent.
export type MaintenanceRuleTemplate = {
  id: string;
  /// Prefilled rule name. Deliberately not derived from the localized title:
  /// the name is an operator-facing identifier that should not change shape
  /// when the UI language does.
  name: string;
  titleKey: string;
  descriptionKey: string;
  actionKind: MaintenanceActionKind;
  /// Prefilled target profile for a profile-changing action. Left empty on
  /// every shipped template: which profile to move a title to is a choice only
  /// the operator can make, so the editor asks for it before the rule saves.
  targetQualityProfileId?: string;
  /// Prefilled tag labels for a tagging action. A template may name a label
  /// that does not exist in the registry yet; the editor shows it as a picked
  /// chip and the API refuses to save until an administrator defines it, which
  /// is the correct order — the vocabulary is an administrator's decision.
  tags?: string[];
  /// True when the template's action needs a target quality profile the
  /// operator still has to pick. Declared on the template rather than read from
  /// the action descriptors, because the gallery is static UI that renders the
  /// same whether or not the descriptor query has answered.
  requiresTargetQualityProfile?: boolean;
  graceDays: number;
  subjectFacets: MaintenanceTemplateFacet[];
  /// True when the template's action deletes files. The gallery marks these,
  /// and their copy says so, even though the instance gates and per-rule arming
  /// still have to agree before anything is removed.
  destructive?: boolean;
  regoSource: string;
};

export const MAINTENANCE_RULE_TEMPLATES: MaintenanceRuleTemplate[] = [
  {
    id: "dead-wanted-entries",
    name: "dead_wanted_entries",
    titleKey: "settings.maintenanceTemplateDeadWantedTitle",
    descriptionKey: "settings.maintenanceTemplateDeadWantedDescription",
    actionKind: "UNMONITOR_SCOPE_KEEP_FILES",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.monitored\n\tnot input.facts.has_file\n}\n",
  },
  {
    id: "library-aging",
    name: "library_aging",
    titleKey: "settings.maintenanceTemplateLibraryAgingTitle",
    descriptionKey: "settings.maintenanceTemplateLibraryAgingDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 180,
    subjectFacets: ["movie"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.has_file\n\tnot \"keep\" in input.facts.tags\n}\n",
  },
  {
    id: "added-over-180-days-ago",
    name: "added_over_180_days_ago",
    titleKey: "settings.maintenanceTemplateAddedLongAgoTitle",
    descriptionKey: "settings.maintenanceTemplateAddedLongAgoDescription",
    actionKind: "UNMONITOR_SCOPE_KEEP_FILES",
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      "package rules\nimport rego.v1\n\nday_ns := (24 * 60 * 60) * 1000000000\n\nmatch if {\n\tage := time.parse_rfc3339_ns(input.evaluation_time) - time.parse_rfc3339_ns(input.facts.added_at)\n\tage > 180 * day_ns\n}\n",
  },
  {
    id: "oversized-releases",
    name: "oversized_releases",
    titleKey: "settings.maintenanceTemplateOversizedTitle",
    descriptionKey: "settings.maintenanceTemplateOversizedDescription",
    actionKind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    requiresTargetQualityProfile: true,
    graceDays: 0,
    subjectFacets: ["movie"],
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if input.facts.total_file_size_bytes > 40000000000\n",
  },
  {
    id: "four-k-purge",
    name: "four_k_purge",
    titleKey: "settings.maintenanceTemplateFourKPurgeTitle",
    descriptionKey: "settings.maintenanceTemplateFourKPurgeDescription",
    actionKind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    requiresTargetQualityProfile: true,
    graceDays: 7,
    subjectFacets: ["movie"],
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n",
  },
  {
    id: "requested-media-expiry",
    name: "requested_media_expiry",
    titleKey: "settings.maintenanceTemplateRequestedExpiryTitle",
    descriptionKey: "settings.maintenanceTemplateRequestedExpiryDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 120,
    subjectFacets: ["movie", "show"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.requested\n\tnot \"keep\" in input.facts.tags\n}\n",
  },
  {
    id: "departed-requester",
    name: "departed_requester",
    titleKey: "settings.maintenanceTemplateDepartedRequesterTitle",
    descriptionKey: "settings.maintenanceTemplateDepartedRequesterDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 30,
    subjectFacets: ["movie", "show"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\t\"departed-user\" in input.facts.requested_by_usernames\n}\n",
  },
  {
    id: "system-added-cleanup",
    name: "system_added_cleanup",
    titleKey: "settings.maintenanceTemplateSystemAddedTitle",
    descriptionKey: "settings.maintenanceTemplateSystemAddedDescription",
    actionKind: "UNMONITOR_SCOPE_KEEP_FILES",
    graceDays: 60,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tnot input.facts.added_by_user_id\n\tinput.facts.has_file\n}\n",
  },
  {
    id: "watched-by-every-requester",
    name: "watched_by_every_requester",
    titleKey: "settings.maintenanceTemplateWatchedRequestedTitle",
    descriptionKey: "settings.maintenanceTemplateWatchedRequestedDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 30,
    subjectFacets: ["movie"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.requested\n\tinput.facts.watched_by_all_requesters\n}\n",
  },
  {
    id: "expired-request-leases",
    name: "expired_request_leases",
    titleKey: "settings.maintenanceTemplateExpiredLeasesTitle",
    descriptionKey: "settings.maintenanceTemplateExpiredLeasesDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 7,
    subjectFacets: ["movie", "show"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n\tnot input.facts.keep_claim_active\n}\n",
  },
  {
    id: "tagged-for-removal",
    name: "tagged_for_removal",
    titleKey: "settings.maintenanceTemplateTaggedForRemovalTitle",
    descriptionKey: "settings.maintenanceTemplateTaggedForRemovalDescription",
    actionKind: "DELETE_TITLE_AND_FILES",
    graceDays: 7,
    subjectFacets: ["movie", "show"],
    destructive: true,
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if {\n\t\"remove\" in input.facts.tags\n}\n",
  },
  {
    id: "flag-for-review",
    name: "flag_for_review",
    titleKey: "settings.maintenanceTemplateFlagForReviewTitle",
    descriptionKey: "settings.maintenanceTemplateFlagForReviewDescription",
    actionKind: "ADD_TAGS",
    tags: ["needs-review"],
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nday_ns := (24 * 60 * 60) * 1000000000\n\nmatch if {\n\tinput.facts.has_file\n\tage := time.parse_rfc3339_ns(input.evaluation_time) - time.parse_rfc3339_ns(input.facts.first_imported_at)\n\tage > 365 * day_ns\n}\n",
  },
  {
    id: "no-quality-profile",
    name: "no_quality_profile",
    titleKey: "settings.maintenanceTemplateNoProfileTitle",
    descriptionKey: "settings.maintenanceTemplateNoProfileDescription",
    actionKind: "DO_NOTHING",
    graceDays: 0,
    subjectFacets: ["movie", "show"],
    regoSource:
      "package rules\nimport rego.v1\n\nmatch if not input.facts.quality_profile_id\n",
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
