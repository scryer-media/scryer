import type {
  ArrSource,
  Diagnostic,
  ImportedCustomFormat,
} from "@/lib/arr-custom-format/types";

const MIN_I32 = -2_147_483_648;
const MAX_I32 = 2_147_483_647;

function validScore(value: number): boolean {
  return Number.isSafeInteger(value) && value >= MIN_I32 && value <= MAX_I32;
}

export function candidateScores(format: ImportedCustomFormat): number[] {
  return [...new Set(Object.values(format.suggestedScores).filter(validScore))];
}

export function hasScoreConflict(diagnostics: Diagnostic[], formatId: string): boolean {
  return diagnostics.some(
    (diagnostic) => diagnostic.code === "arr.score.conflict" && diagnostic.formatId === formatId,
  );
}

export function defaultScore(
  format: ImportedCustomFormat,
  diagnostics: Diagnostic[],
): string {
  const recommendation = recommendScore(format, diagnostics);
  return recommendation ? String(recommendation.value) : "";
}

export type ScoreRecommendation = {
  value: number;
  reasonKey:
    | "settings.arrImportScoreReasonExported"
    | "settings.arrImportScoreReasonDefault"
    | "settings.arrImportScoreReasonPreference"
    | "settings.arrImportScoreReasonPenalty"
    | "settings.arrImportScoreReasonExtras";
};

/** Advisory only: exported weights win; names must express clear intent.
 * Never infer a preference from an implementation, regex, or negate flag.
 */
export function recommendScore(
  format: ImportedCustomFormat,
  diagnostics: Diagnostic[],
): ScoreRecommendation | null {
  if (hasScoreConflict([...diagnostics, ...(format.inspectionDiagnostics ?? [])], format.id)) return null;
  const candidates = candidateScores(format);
  if (candidates.length === 1) return { value: candidates[0]!, reasonKey: "settings.arrImportScoreReasonExported" };
  const exportedDefault = format.suggestedScores.default;
  if (candidates.length > 1 && validScore(exportedDefault)) return { value: exportedDefault, reasonKey: "settings.arrImportScoreReasonDefault" };
  // Ambiguous/invalid exported scores still need an explicit user choice.
  if (Object.keys(format.suggestedScores).length) return null;
  if (!format.specifications.length || [...diagnostics, ...(format.inspectionDiagnostics ?? [])].some((item) => item.level === "error" && item.formatId === format.id)) return null;

  const name = format.name.toLowerCase().replace(/[._-]+/g, " ").replace(/\s+/g, " ").trim();
  if (/^(?:prefer(?:red)?|boost|favor(?:ite)?|favour(?:ite)?|reward)\b/.test(name)) {
    return { value: 100, reasonKey: "settings.arrImportScoreReasonPreference" };
  }
  // Only plain, positive extras/sample conditions carry this stronger penalty.
  // "No extras" and negated-only conditions do not imply unwanted content.
  if (/^(?:(?:avoid|block|exclude|unwanted) )?(?:extras?|samples?)$/.test(name) && format.specifications.every((spec) => !spec.negate)) {
    return { value: -1000, reasonKey: "settings.arrImportScoreReasonExtras" };
  }
  if (/^(?:avoid|penali[sz]e|undesirable|unwanted|bad|low quality|lq|block|reject|exclude)\b/.test(name)) {
    return { value: -100, reasonKey: "settings.arrImportScoreReasonPenalty" };
  }
  return null;
}

export function sourceFacets(source: ArrSource): string[] {
  return source === "radarr" ? ["movie"] : ["series", "anime"];
}

export function collectFormatScores(
  formats: ImportedCustomFormat[],
  scores: Record<string, string>,
): { scores: Record<string, number> } | { missingFormat: ImportedCustomFormat } {
  const result: Record<string, number> = {};
  for (const format of formats) {
    const value = scores[format.id]?.trim() ?? "";
    if (!/^-?\d+$/.test(value)) {
      return { missingFormat: format };
    }
    const score = Number(value);
    if (!validScore(score)) {
      return { missingFormat: format };
    }
    result[format.id] = score;
  }
  return { scores: result };
}

/** Worker responses are ignored after cancellation, a retry, or newer input. */
export function isCurrentTranslationRequest(activeRequestId: number, responseRequestId: number) {
  return activeRequestId === responseRequestId;
}
