import type {
  ArrSource,
  Diagnostic,
  ImportedCustomFormat,
} from "@/lib/arr-custom-format/types";

const MIN_I32 = -2_147_483_648;
const MAX_I32 = 2_147_483_647;

export function candidateScores(format: ImportedCustomFormat): number[] {
  return [...new Set(Object.values(format.suggestedScores))];
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
  if (hasScoreConflict(diagnostics, format.id)) return "";
  const candidates = candidateScores(format);
  return candidates.length === 1 ? String(candidates[0]) : "";
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
    if (!Number.isSafeInteger(score) || score < MIN_I32 || score > MAX_I32) {
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
