export type ScoringEntryKind = "score_contribution" | "mandatory_rejection" | "final_score_rejection";

export type ReleaseDecisionExplanationEntry = {
  code: string;
  delta: number;
  kind?: ScoringEntryKind;
};

export function scoringEntryText(entry: { delta: number; kind?: ScoringEntryKind }, t: (key: string) => string): string {
  if (entry.kind === "mandatory_rejection") return t("scoring.mandatoryRejection");
  if (entry.kind === "final_score_rejection") return t("scoring.finalScoreRejection");
  return entry.delta > 0 ? `+${entry.delta}` : String(entry.delta);
}

export function parseDecisionExplanation(
  explanationJson: unknown,
): ReleaseDecisionExplanationEntry[] {
  const scoringLog = Array.isArray(explanationJson)
    ? explanationJson
    : nestedScoringLog(explanationJson);

  return scoringLog.flatMap((entry) => {
    if (
      !entry ||
      typeof entry !== "object" ||
      !("code" in entry) ||
      typeof entry.code !== "string" ||
      entry.code.trim().length === 0 ||
      !("delta" in entry) ||
      typeof entry.delta !== "number" ||
      !Number.isFinite(entry.delta)
    ) {
      return [];
    }

    const kind = "kind" in entry && (entry.kind === "score_contribution" || entry.kind === "mandatory_rejection" || entry.kind === "final_score_rejection") ? entry.kind : undefined;
    return [{ code: entry.code, delta: entry.delta, ...(kind ? { kind } : {}) }];
  });
}

function nestedScoringLog(explanationJson: unknown): unknown[] {
  if (!explanationJson || typeof explanationJson !== "object") return [];
  if (!("quality_profile_decision" in explanationJson)) return [];

  const qualityDecision = explanationJson.quality_profile_decision;
  if (!qualityDecision || typeof qualityDecision !== "object") return [];
  if (!("scoring_log" in qualityDecision)) return [];

  return Array.isArray(qualityDecision.scoring_log)
    ? qualityDecision.scoring_log
    : [];
}
