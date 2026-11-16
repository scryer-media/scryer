import type {
  LifecycleClaimState,
  MediaRequestLeaseRecord,
  RequestDecisionOutcome,
  RequestDeclaredActions,
  RequestRuleSetDetail,
  RequestRuleSetDraft,
  RequestShaping,
  RequestVote,
} from "@/lib/types/request-rule-sets";
import {
  MONITOR_TYPE_LABEL_KEYS,
  defaultMonitorTypesForFacet,
} from "./default-monitor-type.ts";
import type { RuleBuilderSchema } from "./rule-builder.ts";
import {
  attachedRuleBuilderModel,
  ruleBuilderModelToSave,
  withNewRuleBuilderModel,
  type RuleBuilderBinding,
} from "./rule-builder-mode.ts";

/// How many stored decisions the recent-decisions table asks for. The API
/// clamps its own maximum at 500 and defaults to 50; this is only how much of
/// it the panel wants.
export const REQUEST_DECISION_LIMIT = 50;

/// Lease windows the request dialog offers before it falls back to a number the
/// requester types. Forever is the first option and the default, because that
/// is what Scryer granted before leases existed.
export const REQUEST_LEASE_DAY_CHOICES = [7, 14, 30, 60, 90] as const;
export const REQUEST_LEASE_DAYS_MIN = 1;
export const REQUEST_LEASE_DAYS_MAX = 3650;

/// Starter matcher for a new rule. No `package` or `import` line: the API
/// generates both and strips them back off when it hands the source to the
/// editor, so a template carrying them would vanish on the first round trip.
export const REQUEST_STARTER_SOURCE = `# Request rule. Vote with \`approve\`, \`deny\` or \`manual\` as
# \`name if { ... }\`. Shape the approval with plain literals:
# set_quality_profile := "Profile name", set_lease_days := 30 (0 is forever),
# set_monitor := "FUTURE_EPISODES", set_required_audio_languages := ["eng"] and
# set_preferred_audio_languages := ["jpn", "eng"]. add_tags := ["tag"] tags
# the title and notify := "text" sends a notification. A rule needs a vote, a
# shaping value or add_tags; \`reasons\` explains the choice.
#
# An auto-approved request gets the shaped values; one held for review shows
# them to the approver as suggestions. Two rules setting different values for
# the same thing send the request to review.
#
# Facts are plain values: \`input.facts.is_adult\` is the boolean itself. A fact
# Scryer could not observe is simply missing, so a rule that reads it holds the
# request for a person instead of guessing — you never write that guard.

approve if {
	input.facts.certification_rank <= 2
}

manual if {
	input.facts.is_adult
}

set_lease_days := 30 if {
	input.facts.is_adult
}
`;

// ── Action heads ─────────────────────────────────────────────────────────
// A head's label, order and help come from the served output descriptors.

/// A monitoring value a rule may set. ADVANCED is a per-title season choice,
/// so no rule can set it.
export const REQUEST_SHAPING_MONITOR_TYPES = [
  ...defaultMonitorTypesForFacet("MOVIE"),
  ...defaultMonitorTypesForFacet("SERIES"),
] as const;

/// Translation key for a monitoring value, or null for one this build does
/// not know.
export function requestMonitorTypeLabelKey(value: string): string | null {
  return (
    MONITOR_TYPE_LABEL_KEYS[value as keyof typeof MONITOR_TYPE_LABEL_KEYS] ??
    null
  );
}

/// One head as a summary shows it: the head and the values it carries, already
/// formatted for display. Vote heads carry no value.
export type RequestActionEntry = {
  head: string;
  values: string[];
};

/// How a summary formats the values it cannot show raw.
export type RequestActionValueFormat = {
  /// `0` is forever.
  leaseDays: (days: number) => string;
  monitor: (value: string) => string;
  languages: (codes: string[]) => string;
};

/// Everything a rule's source can do, as the server lists it, with every
/// literal each valued head can produce. Only the heads the API carries values
/// for in a field of their own get values; any other head is listed bare.
export function requestDeclaredActionEntries(
  declared: RequestDeclaredActions | null | undefined,
  format: RequestActionValueFormat,
): RequestActionEntry[] {
  if (!declared) return [];
  const languages = (codes: string[]) =>
    codes.length > 0 ? [format.languages(codes)] : [];
  const values: Record<string, string[]> = {
    SET_QUALITY_PROFILE: declared.qualityProfiles,
    SET_LEASE_DAYS: declared.leaseDays.map(format.leaseDays),
    SET_MONITOR: declared.monitors.map(format.monitor),
    SET_REQUIRED_AUDIO_LANGUAGES: languages(declared.requiredAudioLanguages),
    SET_PREFERRED_AUDIO_LANGUAGES: languages(declared.preferredAudioLanguages),
    ADD_TAGS: declared.addTags,
    NOTIFY: declared.notifications,
  };
  return [...new Set(declared.heads)].map((head) => ({
    head,
    values: [...(values[head] ?? [])],
  }));
}

/// One shaped value as a decision shows it: the head, the formatted value and
/// the rule that set it.
export type RequestShapingEntry = {
  head: string;
  value: string;
  /// Null where the rule's name was not selected.
  ruleSetName: string | null;
};

/// What an approval would be changed to. Conflicting heads are not here;
/// `shaping.conflicts` lists them.
export function requestShapingEntries(
  shaping: RequestShaping | null | undefined,
  format: RequestActionValueFormat,
): RequestShapingEntry[] {
  if (!shaping) return [];
  const entries: RequestShapingEntry[] = [];
  const push = (
    head: string,
    shaped: { value: unknown; ruleSetName?: string } | null | undefined,
    value: (shaped: never) => string,
  ) => {
    if (shaped)
      entries.push({
        head,
        value: value(shaped.value as never),
        ruleSetName: shaped.ruleSetName ?? null,
      });
  };
  push("SET_QUALITY_PROFILE", shaping.qualityProfile, (value: string) => value);
  push("SET_LEASE_DAYS", shaping.leaseDays, format.leaseDays);
  push("SET_MONITOR", shaping.monitor, format.monitor);
  push(
    "SET_REQUIRED_AUDIO_LANGUAGES",
    shaping.requiredAudioLanguages,
    format.languages,
  );
  push(
    "SET_PREFERRED_AUDIO_LANGUAGES",
    shaping.preferredAudioLanguages,
    format.languages,
  );
  for (const notification of shaping.notifications ?? [])
    push("NOTIFY", notification, (value: string) => value);
  return entries;
}

/// The conflicting heads, each once, as the server listed them.
export function requestShapingConflicts(
  shaping: RequestShaping | null | undefined,
): string[] {
  if (!shaping) return [];
  return [...new Set(shaping.conflicts)];
}

/// What the approver's form starts from when rules suggested values for a
/// request held for review. Only fields a rule set are present; `suggested`
/// maps each head the form marks as suggested to the rule that suggested it.
export type RequestApprovalPrefill = {
  qualityProfileId?: string;
  monitorType?: string;
  /// `forever` for a lease of 0 days.
  lease?: { forever: true } | { forever: false; days: number };
  requiredAudioLanguages?: string[];
  preferredAudioLanguages?: string[];
  suggested: Partial<Record<string, string>>;
};

/// Read a decision's shaping into the approver's form. Applies only to a
/// request held for review: an auto-approved request already got the values,
/// and a denied one shows them in its trace only. A profile name that matches
/// no offered profile, or a monitoring value the request's facet does not
/// offer, is left out rather than guessed. Each suggested head maps to the
/// rule set that suggested it.
export function requestApprovalPrefill(
  decision:
    | { effectiveOutcome: string; shaping?: RequestShaping | null }
    | null
    | undefined,
  facet: string,
  qualityProfiles: readonly { id: string; name: string }[],
): RequestApprovalPrefill {
  const prefill: RequestApprovalPrefill = { suggested: {} };
  const shaping = decision?.shaping;
  if (!shaping || decision.effectiveOutcome !== "MANUAL_REVIEW") return prefill;
  if (shaping.qualityProfile) {
    const profile = qualityProfiles.find(
      (candidate) => candidate.name === shaping.qualityProfile?.value,
    );
    if (profile) {
      prefill.qualityProfileId = profile.id;
      prefill.suggested.SET_QUALITY_PROFILE =
        shaping.qualityProfile.ruleSetName ?? "";
    }
  }
  if (
    shaping.monitor &&
    (defaultMonitorTypesForFacet(facet) as readonly string[]).includes(
      shaping.monitor.value,
    )
  ) {
    prefill.monitorType = shaping.monitor.value;
    prefill.suggested.SET_MONITOR = shaping.monitor.ruleSetName ?? "";
  }
  if (shaping.leaseDays) {
    const days = shaping.leaseDays.value;
    if (days === 0) prefill.lease = { forever: true };
    else if (
      Number.isInteger(days) &&
      days >= REQUEST_LEASE_DAYS_MIN &&
      days <= REQUEST_LEASE_DAYS_MAX
    )
      prefill.lease = { forever: false, days };
    if (prefill.lease)
      prefill.suggested.SET_LEASE_DAYS = shaping.leaseDays.ruleSetName ?? "";
  }
  if (shaping.requiredAudioLanguages) {
    prefill.requiredAudioLanguages = [...shaping.requiredAudioLanguages.value];
    prefill.suggested.SET_REQUIRED_AUDIO_LANGUAGES =
      shaping.requiredAudioLanguages.ruleSetName ?? "";
  }
  if (shaping.preferredAudioLanguages) {
    prefill.preferredAudioLanguages = [
      ...shaping.preferredAudioLanguages.value,
    ];
    prefill.suggested.SET_PREFERRED_AUDIO_LANGUAGES =
      shaping.preferredAudioLanguages.ruleSetName ?? "";
  }
  return prefill;
}

/// How request drafts meet the builder. A request rule has no variant.
export type RequestRuleBuilderBinding = RuleBuilderBinding<RequestRuleSetDraft>;

/// `schema` is null until the served outputs are read.
export function requestRuleBuilder(
  schema: RuleBuilderSchema | null,
): RequestRuleBuilderBinding {
  return { schema, variant: () => null, starterSource: REQUEST_STARTER_SOURCE };
}

/// A draft written by hand: the starter source and no builder model.
export function initialRequestRuleDraft(): RequestRuleSetDraft {
  return {
    name: "",
    description: "",
    regoSource: REQUEST_STARTER_SOURCE,
    libraryIds: [],
    builderModel: null,
  };
}

/// A new rule opens in the builder with nothing chosen yet.
export function newRequestRuleBuilderDraft(
  binding: RequestRuleBuilderBinding,
): RequestRuleSetDraft {
  return withNewRuleBuilderModel(binding, initialRequestRuleDraft());
}

/// The editor draft for a saved rule. Its stored builder model counts only
/// while it still regenerates the stored source.
export function requestRuleDraftFromDetail(
  detail: RequestRuleSetDetail,
  binding: RequestRuleBuilderBinding,
): RequestRuleSetDraft {
  return {
    name: detail.ruleSet.name,
    description: detail.ruleSet.description ?? "",
    regoSource: detail.revision.regoSource,
    libraryIds: [...detail.ruleSet.libraryIds],
    builderModel: attachedRuleBuilderModel(
      binding.schema,
      detail.builderModel,
      null,
      detail.revision.regoSource,
    ),
  };
}

/// A copy is a new rule, so it opens with a distinct name and no id. The
/// matcher and the library scope come across verbatim, which is the whole point
/// of copying one, and a rule still made by the builder stays in it.
export function copyRequestRuleDraft(
  detail: RequestRuleSetDetail,
  binding: RequestRuleBuilderBinding,
): RequestRuleSetDraft {
  const base = requestRuleDraftFromDetail(detail, binding);
  return { ...base, name: `${base.name}_copy` };
}

export function createRequestRuleSetInput(
  draft: RequestRuleSetDraft,
  binding: RequestRuleBuilderBinding,
) {
  return {
    name: draft.name.trim(),
    description: draft.description.trim() || null,
    regoSource: draft.regoSource,
    libraryIds: draft.libraryIds,
    builderModel: ruleBuilderModelToSave(binding, draft),
  };
}

/// Without a schema nothing could be checked, so `builderModel` is left out
/// and the stored model stays as it is.
export function updateRequestRuleSourceInput(
  ruleSetId: string,
  draft: RequestRuleSetDraft,
  binding: RequestRuleBuilderBinding,
) {
  return {
    ruleSetId,
    regoSource: draft.regoSource,
    ...(binding.schema
      ? { builderModel: ruleBuilderModelToSave(binding, draft) }
      : {}),
  };
}

export function updateRequestRuleMetadataInput(
  ruleSetId: string,
  draft: RequestRuleSetDraft,
) {
  return {
    ruleSetId,
    name: draft.name.trim(),
    description: draft.description.trim() || null,
    libraryIds: draft.libraryIds,
  };
}

// ── Naming people in a matcher ───────────────────────────────────────────

/// A Rego string literal for one username. Only the two characters Rego escapes
/// inside a double-quoted string are touched; a username carrying anything else
/// is written through unchanged, because rewriting it would name a different
/// person than the one the operator picked.
function regoStringLiteral(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

const REQUESTERS_SET_PATTERN = /(requesters\s*:=\s*\{)[^}]*(\})/;
const USERNAME_LITERAL_PATTERN =
  /(input\.requester\.username\s*)(==|in)(\s*)(\{[^}]*\}|"(?:[^"\\]|\\.)*")/;

/// Whether a matcher has somewhere for the user picker to write. A rule that
/// names nobody has no set and no literal, and rewriting it would mean guessing
/// where the author wanted the names.
export function requestRuleNamesRequesters(source: string): boolean {
  return (
    REQUESTERS_SET_PATTERN.test(source) || USERNAME_LITERAL_PATTERN.test(source)
  );
}

/// Rewrite whichever way a matcher names people with the usernames the picker
/// selected. The `requesters := {…}` set wins when both shapes are present,
/// because that is the form the shipped templates use and the one an author
/// edits deliberately. A single `== "name"` comparison becomes an `in {…}` set
/// membership once more than one person is selected, which is the only honest
/// way to say "any of these" with an equality operator.
///
/// Returns null when there is nothing to rewrite, so the caller can say so
/// rather than silently leaving the matcher naming the template's placeholder.
export function applyRequestersToSource(
  source: string,
  usernames: string[],
): string | null {
  const names = usernames.map((name) => name.trim()).filter(Boolean);
  const literals = names.map(regoStringLiteral);

  if (REQUESTERS_SET_PATTERN.test(source)) {
    return source.replace(
      REQUESTERS_SET_PATTERN,
      (_match, open: string, close: string) =>
        `${open}${literals.join(", ")}${close}`,
    );
  }

  if (USERNAME_LITERAL_PATTERN.test(source)) {
    return source.replace(
      USERNAME_LITERAL_PATTERN,
      (_match, head: string, _operator: string, space: string) =>
        literals.length === 1
          ? `${head}==${space}${literals[0]}`
          : `${head}in${space}{${literals.join(", ")}}`,
    );
  }

  return null;
}

// ── Labels and tones ─────────────────────────────────────────────────────

const DECISION_OUTCOME_LABEL_KEYS: Record<RequestDecisionOutcome, string> = {
  AUTO_APPROVE: "requests.decisionOutcomeAutoApprove",
  MANUAL_REVIEW: "requests.decisionOutcomeManualReview",
  DENY: "requests.decisionOutcomeDeny",
};

export function requestDecisionOutcomeLabelKey(outcome: string): string | null {
  return DECISION_OUTCOME_LABEL_KEYS[outcome as RequestDecisionOutcome] ?? null;
}

export function requestDecisionOutcomeBadgeTone(
  outcome: string,
): "neutral" | "positive" | "warning" | "negative" {
  switch (outcome) {
    case "AUTO_APPROVE":
      return "positive";
    case "DENY":
      return "negative";
    case "MANUAL_REVIEW":
      return "warning";
    default:
      return "neutral";
  }
}

const VOTE_LABEL_KEYS: Record<RequestVote, string> = {
  APPROVE: "requests.voteApprove",
  DENY: "requests.voteDeny",
  MANUAL: "requests.voteManual",
  ABSTAIN: "requests.voteAbstain",
};

export function requestVoteLabelKey(vote: string | null): string | null {
  if (!vote) {
    return "requests.voteNone";
  }
  return VOTE_LABEL_KEYS[vote as RequestVote] ?? null;
}

export function requestVoteBadgeTone(
  vote: string | null,
): "neutral" | "positive" | "warning" | "negative" {
  switch (vote) {
    case "APPROVE":
      return "positive";
    case "DENY":
      return "negative";
    case "MANUAL":
      return "warning";
    default:
      return "neutral";
  }
}

/// Fallback reasons the arbitration records when no rule decided outright.
/// Anything this build does not know renders as the raw code, which is more
/// useful than hiding it.
const FALLBACK_REASON_LABEL_KEYS: Record<string, string> = {
  rule_manual: "requests.fallbackRuleManual",
  held: "requests.fallbackHeld",
  error: "requests.fallbackError",
  no_rule_matched: "requests.fallbackNoRuleMatched",
  library_permission: "requests.fallbackLibraryPermission",
  shaping_conflict: "requests.fallbackShapingConflict",
  shaping_invalid: "requests.fallbackShapingInvalid",
};

export function requestFallbackReasonLabelKey(reason: string): string | null {
  return FALLBACK_REASON_LABEL_KEYS[reason] ?? null;
}

/// Whether a fallback reason tells the requester something they can act on. A
/// rule that held for missing metadata is worth saying out loud; "no rule
/// matched" only means the request took the ordinary path.
export function requestFallbackReasonIsInformative(
  reason: string | null,
): boolean {
  return reason !== null && reason !== "no_rule_matched";
}

const CLAIM_STATE_LABEL_KEYS: Record<LifecycleClaimState, string> = {
  DORMANT: "requests.claimStateDormant",
  ACTIVE: "requests.claimStateActive",
  EXPIRED: "requests.claimStateExpired",
  RELEASED: "requests.claimStateReleased",
  CONVERTED: "requests.claimStateConverted",
};

export function titleClaimStateLabelKey(state: string): string | null {
  return CLAIM_STATE_LABEL_KEYS[state as LifecycleClaimState] ?? null;
}

export function titleClaimStateBadgeTone(
  state: string,
): "neutral" | "positive" | "warning" | "negative" {
  switch (state) {
    case "ACTIVE":
      return "positive";
    case "DORMANT":
      return "warning";
    case "EXPIRED":
      return "negative";
    default:
      return "neutral";
  }
}

// ── Lease badges ─────────────────────────────────────────────────────────

/// What one request's lease is doing right now. `requested` is a window nobody
/// has granted yet, `dormant` a granted one waiting for the title's first
/// import — "expires in N days" is meaningless until then, so the badge says
/// "N days from first import" instead.
export type RequestLeaseBadge =
  | { variant: "forever" }
  | { variant: "requested"; days: number }
  | { variant: "dormant"; days: number }
  | { variant: "active"; expiresAt: string }
  | { variant: "expired" }
  | { variant: "released" };

export function requestLeaseBadge(request: {
  requestedLeaseDays?: number | null;
  approvedLeaseDays?: number | null;
  lease?: MediaRequestLeaseRecord | null;
}): RequestLeaseBadge {
  const lease = request.lease ?? null;
  if (!lease) {
    const days = request.requestedLeaseDays ?? null;
    return days === null || days <= 0
      ? { variant: "forever" }
      : { variant: "requested", days };
  }

  const days = lease.approvedDays ?? lease.requestedDays ?? null;
  switch (lease.state) {
    case "ACTIVE":
      return lease.expiresAt
        ? { variant: "active", expiresAt: lease.expiresAt }
        : { variant: "forever" };
    case "DORMANT":
      return days === null || days <= 0
        ? { variant: "forever" }
        : { variant: "dormant", days };
    case "EXPIRED":
      return { variant: "expired" };
    default:
      return { variant: "released" };
  }
}

export function requestLeaseBadgeTone(
  badge: RequestLeaseBadge,
): "neutral" | "positive" | "warning" | "negative" | "info" {
  switch (badge.variant) {
    case "forever":
      return "positive";
    case "active":
      return "info";
    case "dormant":
      return "warning";
    case "requested":
      return "neutral";
    case "expired":
      return "negative";
    default:
      return "neutral";
  }
}

/// Clamp a typed lease length into the window the API accepts. A blank or
/// unparseable entry becomes the smallest legal lease rather than forever,
/// because forever is a separate choice the requester makes on purpose.
export function clampRequestLeaseDays(days: number): number {
  if (!Number.isFinite(days)) {
    return REQUEST_LEASE_DAYS_MIN;
  }
  return Math.min(
    REQUEST_LEASE_DAYS_MAX,
    Math.max(REQUEST_LEASE_DAYS_MIN, Math.trunc(days)),
  );
}

// ── Filters ──────────────────────────────────────────────────────────────

export const REQUEST_FILTER_ALL = "all";

export function requestFilterArgument(value: string): string | undefined {
  return value === REQUEST_FILTER_ALL || !value ? undefined : value;
}

/** Permission-sensitive targeting is explicitly named by validation. */
export function isPersonTargetingRefusal(diagnostic: {
  code: string;
}): boolean {
  return diagnostic.code === "RULE_TARGETS_PERSON";
}

const REQUESTER_REFERENCE_PATTERN =
  /\binput\s*(?:\.\s*requester\b|\[\s*"requester"\s*\])/;

/// Whether a matcher reads requester fields, which the API only saves for an
/// author who can manage permissions. Comments are ignored so a note about the
/// requester does not raise the hint; the server's check stays authoritative.
export function requestRuleReadsRequester(source: string): boolean {
  return source
    .split("\n")
    .some((line) => REQUESTER_REFERENCE_PATTERN.test(line.replace(/#.*$/, "")));
}
