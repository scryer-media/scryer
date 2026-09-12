import type {
  LifecycleClaimState,
  MediaRequestLeaseRecord,
  RequestDecisionOutcome,
  RequestEvaluationMode,
  RequestRuleSetDetail,
  RequestRuleSetDraft,
  RequestVote,
} from "@/lib/types/request-rule-sets";

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
export const REQUEST_STARTER_SOURCE = `# Request matcher. Define at least one of \`approve\`, \`deny\`, \`manual\` or
# \`tags\`; a rule that can never vote is refused when you save it.
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
`;

export function initialRequestRuleDraft(): RequestRuleSetDraft {
  return {
    name: "",
    description: "",
    regoSource: REQUEST_STARTER_SOURCE,
    libraryIds: [],
  };
}

export function requestRuleDraftFromDetail(
  detail: RequestRuleSetDetail,
): RequestRuleSetDraft {
  return {
    name: detail.ruleSet.name,
    description: detail.ruleSet.description ?? "",
    regoSource: detail.revision.regoSource,
    libraryIds: [...detail.ruleSet.libraryIds],
  };
}

/// A copy is a new rule, so it opens with a distinct name and no id. The
/// matcher and the library scope come across verbatim, which is the whole point
/// of copying one.
export function copyRequestRuleDraft(
  detail: RequestRuleSetDetail,
): RequestRuleSetDraft {
  const base = requestRuleDraftFromDetail(detail);
  return { ...base, name: `${base.name}_copy` };
}

export function createRequestRuleSetInput(draft: RequestRuleSetDraft) {
  return {
    name: draft.name.trim(),
    description: draft.description.trim() || null,
    regoSource: draft.regoSource,
    libraryIds: draft.libraryIds,
  };
}

export function updateRequestRuleMatcherInput(
  ruleSetId: string,
  draft: RequestRuleSetDraft,
) {
  return { ruleSetId, regoSource: draft.regoSource };
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

export const REQUEST_EVALUATION_MODES: RequestEvaluationMode[] = [
  "DISABLED",
  "SHADOW",
  "ENFORCE",
];

const EVALUATION_MODE_LABEL_KEYS: Record<RequestEvaluationMode, string> = {
  DISABLED: "settings.requestRuleModeDisabled",
  SHADOW: "settings.requestRuleModeShadow",
  ENFORCE: "settings.requestRuleModeEnforce",
};

export function requestEvaluationModeLabelKey(mode: string): string | null {
  return EVALUATION_MODE_LABEL_KEYS[mode as RequestEvaluationMode] ?? null;
}

const EVALUATION_MODE_HELP_KEYS: Record<RequestEvaluationMode, string> = {
  DISABLED: "settings.requestRuleModeDisabledHelp",
  SHADOW: "settings.requestRuleModeShadowHelp",
  ENFORCE: "settings.requestRuleModeEnforceHelp",
};

export function requestEvaluationModeHelpKey(mode: string): string | null {
  return EVALUATION_MODE_HELP_KEYS[mode as RequestEvaluationMode] ?? null;
}

export function requestEvaluationModeBadgeTone(
  mode: string,
): "neutral" | "info" | "warning" | "positive" {
  switch (mode) {
    case "ENFORCE":
      return "positive";
    case "SHADOW":
      return "info";
    default:
      return "neutral";
  }
}

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

/// The API refuses a matcher that reads `input.requester.*` unless the author
/// can manage permissions. The message is the server's, and it is shown
/// verbatim, so this only recognises it well enough to mark the banner.
export function isPersonTargetingRefusal(message: string): boolean {
  return message.toLowerCase().includes("input.requester");
}
