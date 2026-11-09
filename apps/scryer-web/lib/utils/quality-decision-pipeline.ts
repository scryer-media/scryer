import {
  QUALITY_DECISION_GATES,
  qualityDecisionGateDescriptor,
  type KnownQualityDecisionGate,
  type QualityDecisionGate,
  type QualityDecisionGateKind,
} from "../constants/quality-decision-gates.ts";

/** The order a profile uses when it does not set one. */
export const DEFAULT_DECISION_PIPELINE: readonly KnownQualityDecisionGate[] = [
  "QUALITY_TIER",
  "REVISION",
  "PREFERRED_LANGUAGE",
  "OPTIONAL_LANGUAGES",
];

export type DecisionPipelinePresetId =
  "default" | "language_first" | "score_only";

export type DecisionPipelinePreset = {
  id: DecisionPipelinePresetId;
  labelKey: string;
  pipeline: readonly KnownQualityDecisionGate[];
};

export const DECISION_PIPELINE_PRESETS: readonly DecisionPipelinePreset[] = [
  {
    id: "default",
    labelKey: "qualityProfile.decisionPresetDefault",
    pipeline: DEFAULT_DECISION_PIPELINE,
  },
  {
    id: "language_first",
    labelKey: "qualityProfile.decisionPresetLanguageFirst",
    pipeline: [
      "PREFERRED_LANGUAGE",
      "QUALITY_TIER",
      "REVISION",
      "OPTIONAL_LANGUAGES",
    ],
  },
  {
    id: "score_only",
    labelKey: "qualityProfile.decisionPresetScoreOnly",
    pipeline: [],
  },
];

/**
 * Reads a stored or server-sent order. Values are trimmed and upper-cased,
 * empties and repeats dropped, unknown values kept in place. Returns null
 * when the input is not a list, so the caller can pick its own fallback.
 */
export function normalizeDecisionPipeline(
  raw: unknown,
): QualityDecisionGate[] | null {
  if (!Array.isArray(raw)) {
    return null;
  }
  const result: QualityDecisionGate[] = [];
  for (const entry of raw) {
    if (typeof entry !== "string") continue;
    const gate = entry.trim().toUpperCase();
    if (!gate || result.includes(gate)) continue;
    result.push(gate);
  }
  return result;
}

/** The order implied by the proper/repack setting alone. */
export function decisionPipelineForIgnoreProperRepack(
  ignoreProperRepack: boolean,
): QualityDecisionGate[] {
  return DEFAULT_DECISION_PIPELINE.filter(
    (gate) => !ignoreProperRepack || gate !== "REVISION",
  );
}

/** The REVISION step's presence is the proper/repack setting. */
export function ignoresProperRepack(
  pipeline: readonly QualityDecisionGate[],
): boolean {
  return !pipeline.includes("REVISION");
}

export function moveDecisionGate(
  pipeline: readonly QualityDecisionGate[],
  gate: QualityDecisionGate,
  targetIndex: number,
): QualityDecisionGate[] {
  const sourceIndex = pipeline.indexOf(gate);
  if (
    sourceIndex < 0 ||
    targetIndex < 0 ||
    targetIndex >= pipeline.length ||
    sourceIndex === targetIndex
  ) {
    return [...pipeline];
  }
  const reordered = [...pipeline];
  reordered.splice(sourceIndex, 1);
  reordered.splice(targetIndex, 0, gate);
  return reordered;
}

/**
 * Inserts a gate at `targetIndex` (clamped), or at the end when omitted. A
 * gate already in the order is left where it is.
 */
export function addDecisionGate(
  pipeline: readonly QualityDecisionGate[],
  gate: QualityDecisionGate,
  targetIndex?: number,
): QualityDecisionGate[] {
  if (pipeline.includes(gate)) {
    return [...pipeline];
  }
  const index =
    targetIndex === undefined
      ? pipeline.length
      : Math.min(Math.max(targetIndex, 0), pipeline.length);
  const next = [...pipeline];
  next.splice(index, 0, gate);
  return next;
}

export function removeDecisionGate(
  pipeline: readonly QualityDecisionGate[],
  gate: QualityDecisionGate,
): QualityDecisionGate[] {
  return pipeline.filter((entry) => entry !== gate);
}

/** Known gates not in the order, in descriptor-table order. */
export function availableDecisionGates(
  pipeline: readonly QualityDecisionGate[],
): KnownQualityDecisionGate[] {
  return QUALITY_DECISION_GATES.map((gate) => gate.id).filter(
    (gate) => !pipeline.includes(gate),
  );
}

export function decisionPipelinePresetId(
  pipeline: readonly QualityDecisionGate[],
): DecisionPipelinePresetId | null {
  const preset = DECISION_PIPELINE_PRESETS.find(
    (candidate) =>
      candidate.pipeline.length === pipeline.length &&
      candidate.pipeline.every((gate, index) => pipeline[index] === gate),
  );
  return preset?.id ?? null;
}

export type DecisionPipelineNotice =
  /** No tier step: a lower tier can replace a higher one; no tier cutoff. */
  | "tier_absent"
  /** A language step ranks above the tier step. */
  | "tier_below_language"
  /** No revision step: PROPER/REPACK gets no automatic preference. */
  | "revision_absent"
  /** A language step is present; it needs library audio languages. */
  | "language_needs_library_languages";

function gateKind(gate: QualityDecisionGate): QualityDecisionGateKind | null {
  return qualityDecisionGateDescriptor(gate)?.kind ?? null;
}

/** Notices for an order, most serious first. */
export function decisionPipelineNotices(
  pipeline: readonly QualityDecisionGate[],
): DecisionPipelineNotice[] {
  const kinds = pipeline.map(gateKind);
  const tierIndex = kinds.indexOf("tier");
  const firstLanguageIndex = kinds.indexOf("language");
  const notices: DecisionPipelineNotice[] = [];
  if (tierIndex < 0) {
    notices.push("tier_absent");
  } else if (firstLanguageIndex >= 0 && firstLanguageIndex < tierIndex) {
    notices.push("tier_below_language");
  }
  if (!kinds.includes("revision")) {
    notices.push("revision_absent");
  }
  if (firstLanguageIndex >= 0) {
    notices.push("language_needs_library_languages");
  }
  return notices;
}

/** Whether the profile's cutoff tier takes part in decisions. */
export function decisionPipelineUsesTierCutoff(
  pipeline: readonly QualityDecisionGate[],
): boolean {
  return pipeline.some((gate) => gateKind(gate) === "tier");
}

type SummaryTranslate = (
  key: string,
  values?: Record<string, string | number>,
) => string;

/** Short label for a gate, including one this build does not know. */
export function decisionGateLabel(
  gate: QualityDecisionGate,
  t: SummaryTranslate,
): string {
  const descriptor = qualityDecisionGateDescriptor(gate);
  return descriptor
    ? t(descriptor.labelKey)
    : t("qualityProfile.decisionGateUnknown", { value: gate });
}

/** The final step a recorded decision can name besides the order's gates. */
export const SCORE_DECIDING_STEP = "SCORE";

/**
 * "Decided by" text for the step that settled a recorded comparison, or null
 * when nothing was recorded. Score and unknown steps are labelled too.
 */
export function decidedByLabel(
  step: string | null | undefined,
  t: SummaryTranslate,
): string | null {
  const value = typeof step === "string" ? step.trim().toUpperCase() : "";
  if (!value) return null;
  const label =
    value === SCORE_DECIDING_STEP
      ? t("qualityProfile.decisionGateScore")
      : decisionGateLabel(value, t);
  return t("qualityProfile.decidedBy", { step: label });
}

/** One plain-language sentence describing the order, score included. */
export function describeDecisionPipeline(
  pipeline: readonly QualityDecisionGate[],
  t: SummaryTranslate,
): string {
  if (pipeline.length === 0) {
    return t("qualityProfile.decisionSummaryScoreOnly");
  }
  const phrases = pipeline.map((gate) => {
    const descriptor = qualityDecisionGateDescriptor(gate);
    return descriptor
      ? t(descriptor.summaryKey)
      : t("qualityProfile.decisionGateUnknown", { value: gate });
  });
  const steps = phrases
    .slice(1)
    .reduce(
      (previous, next) =>
        t("qualityProfile.decisionSummaryJoin", { previous, next }),
      phrases[0],
    );
  return t("qualityProfile.decisionSummary", { steps });
}
