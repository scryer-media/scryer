export type ReleaseDecisionExplanationEntry = {
  code: string;
  delta: number;
};

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

    return [{ code: entry.code, delta: entry.delta }];
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
