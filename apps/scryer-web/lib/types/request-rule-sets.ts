/// Request rules decide what happens to a media request the moment it is
/// submitted. An enabled rule covering the library decides; with none, the
/// requester's library permission does. A rule is created disabled, so
/// enabling one is always a deliberate second step.

/// Whether an evaluation consulted an enabled rule (`ENFORCE`) or fell back
/// to the requester's permission (`DISABLED`).
export type RequestEvaluationMode = "DISABLED" | "ENFORCE";

/// What a decision came to. `MANUAL_REVIEW` is the domain default, so an
/// unreadable verdict never reads back as an approval or a denial.
export type RequestDecisionOutcome = "AUTO_APPROVE" | "MANUAL_REVIEW" | "DENY";

/// One rule's vote. Null on a vote record means the rule errored rather than
/// abstaining, which is why the wire type is nullable.
export type RequestVote = "APPROVE" | "DENY" | "MANUAL" | "ABSTAIN";

/// What wrote a lifecycle claim: the finite window a request asked for. A
/// request approved without a lease writes no claim.
export type LifecycleClaimProducer = "REQUEST_LEASE";

/// A lifecycle claim is only ever a time to delete.
export type LifecycleClaimKind = "RETAIN_UNTIL";

/// Where a claim is in its life. `DORMANT` is a lease that exists but has not
/// started, because the title has not imported yet; `CONVERTED` is a lease that
/// was replaced by a permanent hold and kept as history.
export type LifecycleClaimState =
  "DORMANT" | "ACTIVE" | "EXPIRED" | "RELEASED" | "CONVERTED";

export type RequestRuleSetRecord = {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  libraryIds: string[];
  currentRevisionNumber: number;
  /// How many stored decisions name this rule set. Read per rule set rather
  /// than batched, so the number on a row is the number the API holds.
  decisionCount: number;
  createdAt: string;
  updatedAt: string;
};

export type RequestRuleRevision = {
  id: string;
  ruleSetId: string;
  revisionNumber: number;
  /// Editor-stripped source: the API removes the package and import lines it
  /// generates, so what round-trips through the editor is what the author wrote.
  regoSource: string;
  matcherContentHash: string;
  createdBy: string | null;
  createdAt: string;
};

export type RequestRuleSetDetail = {
  ruleSet: RequestRuleSetRecord;
  revision: RequestRuleRevision;
};

export type RequestRuleValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
};

/// The whole vocabulary a requester is permitted to see about why a rule
/// decided what it did: a stable code and the name of the rule that raised it.
export type RequestPreflightReason = {
  code: string;
  ruleName: string;
};

/// One rule's contribution to a decision. Only a manager of the request's
/// library ever receives these; a requester reading their own request gets the
/// same decision with `votes` emptied.
export type RequestRuleVoteRecord = {
  ruleSetId: string;
  ruleSetName: string;
  /// `0` on a preview's synthesised vote: a preview result carries no revision
  /// and a guess would be worse than nothing. Never render it as a revision.
  revisionNumber: number;
  vote: RequestVote | null;
  held: boolean;
  reasonCodes: string[];
  tags: string[];
  error: string | null;
};

export type RequestRuleDecisionRecord = {
  /// Null on a preview, which persists no trace and so has nothing to point at.
  id: string | null;
  requestId: string | null;
  evaluatedAt: string;
  mode: RequestEvaluationMode;
  /// What actually happened. `policyOutcome` is what the rules alone came to;
  /// the two differ only when the decision fell back.
  effectiveOutcome: RequestDecisionOutcome;
  policyOutcome: RequestDecisionOutcome;
  fallbackReason: string | null;
  votes: RequestRuleVoteRecord[];
  reasons: RequestPreflightReason[];
  tags: string[];
  inputSchemaVersion: number;
};

export type RequestRulePreviewResult = {
  ruleSetId: string;
  matcherContentHash: string;
  decision: RequestRuleDecisionRecord;
  metadataPartial: boolean;
  /// Of the tags the rule emitted, the ones the tag registry does not define.
  /// They are dropped when a request is decided, so the author has to define
  /// them in Settings before the rule can apply them. `decision.tags` still
  /// lists everything the rule emitted.
  undefinedTags: string[];
  /// The document the matcher actually saw, or null when it could not be
  /// re-parsed. The "why did this not match" affordance.
  inputDocument: unknown;
};

/// What the requester's own pre-flight returns. It has no field a vote could
/// travel in, which is the guarantee rather than a convention.
export type RequestPreflightResult = {
  outcome: RequestDecisionOutcome;
  reasons: RequestPreflightReason[];
  tags: string[];
  metadataPartial: boolean;
  evaluationMode: RequestEvaluationMode;
  /// Why the verdict fell back to needing approval, when it did. It names the
  /// shape of the fallback (`rule_manual`, `held`, `error`, `no_rule_matched`)
  /// and never a rule, so it is safe on the requester's surface.
  fallbackReason: string | null;
};

export type TitleClaimRecord = {
  id: string;
  titleId: string;
  libraryId: string;
  producer: LifecycleClaimProducer;
  producerRef: string | null;
  kind: LifecycleClaimKind;
  state: LifecycleClaimState;
  durationDays: number | null;
  startsAt: string | null;
  expiresAt: string | null;
  createdBy: string | null;
  createdAt: string;
  updatedAt: string;
  releasedReason: string | null;
};

/// The lease holding a request's created title. Null until an approval creates
/// the claim, and `DORMANT` with no window until the title first imports.
export type MediaRequestLeaseRecord = {
  requestedDays: number | null;
  approvedDays: number | null;
  state: LifecycleClaimState;
  startsAt: string | null;
  expiresAt: string | null;
};

/// The metadata a request was decided against, captured at submit time so an
/// approver and the rule that judged them read the same numbers.
export type MediaRequestMetadataRecord = {
  partial: boolean;
  missing: string[];
  genres: string[];
  contentRatings: Array<{ source?: string | null; value?: string | null }>;
  ageRating: number | null;
  certificationLabel: string | null;
  certificationRank: number | null;
  isAdult: boolean;
  tmdbVoteAverage: number | null;
  tmdbVoteCount: number | null;
  popularity: number | null;
  awardCount: number;
};

export type RequestRuleSetDraft = {
  name: string;
  description: string;
  regoSource: string;
  libraryIds: string[];
};

/// One account the user picker can write into a matcher. Only the id and the
/// username matter: a rule names people by username.
export type RequestRuleUserOption = {
  id: string;
  username: string;
};

/// The sample an author previews a matcher against. `leaseForever` and
/// `leaseDays` are mutually exclusive; sending both is refused by the API.
export type RequestRulePreviewSample = {
  userId: string;
  libraryId: string;
  externalIds: Array<{ source: string; value: string }>;
  titleLabel: string;
  qualityProfileId: string;
  monitorType: string;
  leaseForever: boolean;
  leaseDays: number;
};

/// Where a preview run gets its matcher from: the stored revision of a saved
/// rule set, or the unsaved editor draft.
export type RequestRulePreviewSource = "stored" | "draft";
