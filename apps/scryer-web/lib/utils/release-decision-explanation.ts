import { parseScoreLog, type ScoreLogEntry } from "./score-log.ts";

export type ScoringEntryKind =
  "score_contribution" | "mandatory_rejection" | "final_score_rejection";

export type ReleaseDecisionExplanationEntry = ScoreLogEntry;

export function scoringEntryText(
  entry: { delta: number; kind?: ScoringEntryKind },
  t: (key: string) => string,
): string {
  if (entry.kind === "mandatory_rejection")
    return t("scoring.mandatoryRejection");
  if (entry.kind === "final_score_rejection")
    return t("scoring.finalScoreRejection");
  return entry.delta > 0 ? `+${entry.delta}` : String(entry.delta);
}

export function parseDecisionExplanation(
  explanationJson: unknown,
): ReleaseDecisionExplanationEntry[] {
  return parseScoreLog(explanationJson).entries;
}
