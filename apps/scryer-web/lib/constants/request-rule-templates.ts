/// A starter request rule the operator can load into the create-rule editor.
/// Nothing here is ever saved on its own: applying a template prefills the draft
/// and the operator still reviews, names, scopes, and saves it themselves.
///
/// `regoSource` is pinned byte-for-byte against `REQUEST_RULE_EXAMPLES` in
/// `crates/scryer-rules/src/request.rs`, which is the array the API's own
/// fixture test validates. It is written with explicit escapes rather than a
/// template literal that a reformat could quietly re-indent.
export type RequestRuleTemplate = {
  id: string;
  /// Prefilled rule name. Deliberately not derived from the localized title:
  /// the name is an operator-facing identifier that should not change shape
  /// when the UI language does.
  name: string;
  titleKey: string;
  descriptionKey: string;
  /// True when the matcher reads `input.requester.*`. The API refuses to store
  /// one of these unless the author can manage permissions, and the gallery
  /// says so before the operator spends time on it.
  personTargeted?: boolean;
  /// True when the matcher names specific people, which is what the user picker
  /// rewrites. A template without it has nowhere for the picker to write.
  namesRequesters?: boolean;
  regoSource: string;
};

export const REQUEST_RULE_TEMPLATES: RequestRuleTemplate[] = [
  {
    id: "named-requesters-family-rated",
    name: "named_requesters_family_rated",
    titleKey: "settings.requestTemplateNamedRequestersTitle",
    descriptionKey: "settings.requestTemplateNamedRequestersDescription",
    personTargeted: true,
    namesRequesters: true,
    regoSource:
      "package rules\nimport rego.v1\n\nrequesters := {\"alice\", \"bob\", \"carol\"}\n\napprove if {\n\tinput.requester.username in requesters\n\tinput.facts.certification_rank <= 2\n}\n\ntags contains \"family\" if {\n\tinput.facts.certification_rank <= 1\n}\n",
  },
  {
    id: "short-lease-approval",
    name: "short_lease_approval",
    titleKey: "settings.requestTemplateShortLeaseTitle",
    descriptionKey: "settings.requestTemplateShortLeaseDescription",
    personTargeted: true,
    namesRequesters: true,
    regoSource:
      "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"bob\"\n\tnot input.request.lease_forever\n\tinput.request.lease_days <= 14\n}\n",
  },
  {
    id: "low-resolution-approval",
    name: "low_resolution_approval",
    titleKey: "settings.requestTemplateLowResolutionTitle",
    descriptionKey: "settings.requestTemplateLowResolutionDescription",
    personTargeted: true,
    namesRequesters: true,
    regoSource:
      "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"alice\"\n\tinput.facts.quality_profile_max_resolution <= 720\n}\n",
  },
  {
    id: "deny-adult-content",
    name: "deny_adult_content",
    titleKey: "settings.requestTemplateDenyAdultTitle",
    descriptionKey: "settings.requestTemplateDenyAdultDescription",
    regoSource:
      "package rules\nimport rego.v1\n\ndeny if {\n\tinput.facts.is_adult\n}\n\nreasons contains \"adult_content\" if {\n\tinput.facts.is_adult\n}\n",
  },
  {
    id: "monthly-approval-quota",
    name: "monthly_approval_quota",
    titleKey: "settings.requestTemplateMonthlyQuotaTitle",
    descriptionKey: "settings.requestTemplateMonthlyQuotaDescription",
    regoSource:
      "package rules\nimport rego.v1\n\nmanual if {\n\tinput.facts.approved_last_30d >= 5\n}\n",
  },
];
