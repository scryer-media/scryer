import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

export type ArrSource = "sonarr" | "radarr";

export type ArrDiagnostic = {
  code: string;
  severity: "ERROR" | "WARNING" | "INFO";
  message: string;
  location?: string | null;
  formatId?: string | null;
  specificationIndex?: number | null;
};

export type ArrCustomFormatPayload = {
  id: string;
  name: string;
  status: "TRANSLATED" | "DISABLED";
  diagnostics: ArrDiagnostic[];
  suggestedScores: { label: string; score: number }[];
  specifications: { index: number; negate: boolean }[];
  inspectionDiagnostics: ArrDiagnostic[];
};

/** The `translateArrCustomFormats` result. */
export type ArrCustomFormatTranslation = {
  regoSource: string;
  name: string;
  description: string;
  appliedFacets: string[];
  formats: ArrCustomFormatPayload[];
  diagnostics: ArrDiagnostic[];
  inspectionDiagnostics: ArrDiagnostic[];
};

/** A reviewed format, with its suggested scores keyed by export label. */
export type ReviewedCustomFormat = {
  id: string;
  name: string;
  specifications: { negate: boolean }[];
  suggestedScores: Record<string, number>;
  inspectionDiagnostics?: ArrDiagnostic[];
};

export function reviewedFormat(
  format: ArrCustomFormatPayload,
): ReviewedCustomFormat {
  return {
    id: format.id,
    name: format.name,
    specifications: format.specifications,
    suggestedScores: Object.fromEntries(
      format.suggestedScores.map(({ label, score }) => [label, score]),
    ),
    inspectionDiagnostics: format.inspectionDiagnostics,
  };
}

const MIN_I32 = -2_147_483_648;
const MAX_I32 = 2_147_483_647;

function validScore(value: number): boolean {
  return Number.isSafeInteger(value) && value >= MIN_I32 && value <= MAX_I32;
}

export function candidateScores(format: ReviewedCustomFormat): number[] {
  return [...new Set(Object.values(format.suggestedScores).filter(validScore))];
}

export function hasScoreConflict(
  diagnostics: ArrDiagnostic[],
  formatId: string,
): boolean {
  return diagnostics.some(
    (diagnostic) =>
      diagnostic.code === "arr.score.conflict" &&
      diagnostic.formatId === formatId,
  );
}

export function defaultScore(
  format: ReviewedCustomFormat,
  diagnostics: ArrDiagnostic[],
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
  format: ReviewedCustomFormat,
  diagnostics: ArrDiagnostic[],
): ScoreRecommendation | null {
  if (
    hasScoreConflict(
      [...diagnostics, ...(format.inspectionDiagnostics ?? [])],
      format.id,
    )
  )
    return null;
  const candidates = candidateScores(format);
  if (candidates.length === 1)
    return {
      value: candidates[0]!,
      reasonKey: "settings.arrImportScoreReasonExported",
    };
  const exportedDefault = format.suggestedScores.default;
  if (candidates.length > 1 && validScore(exportedDefault))
    return {
      value: exportedDefault,
      reasonKey: "settings.arrImportScoreReasonDefault",
    };
  // Ambiguous/invalid exported scores still need an explicit user choice.
  if (Object.keys(format.suggestedScores).length) return null;
  if (
    !format.specifications.length ||
    [...diagnostics, ...(format.inspectionDiagnostics ?? [])].some(
      (item) => item.severity === "ERROR" && item.formatId === format.id,
    )
  )
    return null;

  const name = format.name
    .toLowerCase()
    .replace(/[._-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (
    /^(?:prefer(?:red)?|boost|favor(?:ite)?|favour(?:ite)?|reward)\b/.test(name)
  ) {
    return { value: 100, reasonKey: "settings.arrImportScoreReasonPreference" };
  }
  // Surround labels are a narrow, explicit audio preference; do not infer from condition patterns.
  if (
    /^(?:5|7) 1 surround$/.test(name) &&
    format.specifications.some((spec) => !spec.negate)
  ) {
    return { value: 100, reasonKey: "settings.arrImportScoreReasonPreference" };
  }
  // Only plain, positive extras/sample conditions carry this stronger penalty.
  // "No extras" and negated-only conditions do not imply unwanted content.
  if (
    /^(?:(?:avoid|block|exclude|unwanted) )?(?:extras?|samples?)$/.test(name) &&
    format.specifications.every((spec) => !spec.negate)
  ) {
    return { value: -1000, reasonKey: "settings.arrImportScoreReasonExtras" };
  }
  if (
    /^(?:avoid|penali[sz]e|undesirable|unwanted|bad|low quality|lq|block|reject|exclude)\b/.test(
      name,
    )
  ) {
    return { value: -100, reasonKey: "settings.arrImportScoreReasonPenalty" };
  }
  return null;
}

export function sourceFacets(source: ArrSource): string[] {
  return source === "radarr" ? ["movie"] : ["series", "anime"];
}

export function collectFormatScores(
  formats: ReviewedCustomFormat[],
  scores: Record<string, string>,
):
  | { scores: { formatId: string; score: number }[] }
  | { missingFormat: ReviewedCustomFormat } {
  const result: { formatId: string; score: number }[] = [];
  for (const format of formats) {
    const value = scores[format.id]?.trim() ?? "";
    if (!/^-?\d+$/.test(value)) {
      return { missingFormat: format };
    }
    const score = Number(value);
    if (!validScore(score)) {
      return { missingFormat: format };
    }
    result.push({ formatId: format.id, score });
  }
  return { scores: result };
}

/** Responses are ignored after cancellation, a retry, or newer input. */
export function isCurrentTranslationRequest(
  activeRequestId: number,
  responseRequestId: number,
) {
  return activeRequestId === responseRequestId;
}

/**
 * The server names why it refused a paste as `extensions.reason` on a
 * `VALIDATION_ERROR`, with the limit it applied as a separate extension.
 */
export const ARR_IMPORT_REFUSAL_KEYS: Readonly<
  Record<string, { key: string; fact?: string }>
> = {
  ARR_CUSTOM_FORMAT_JSON_TOO_LARGE: {
    key: "settings.arrImportRefusalTooLarge",
    fact: "maxBytes",
  },
  ARR_CUSTOM_FORMAT_JSON_INVALID: { key: "settings.arrImportInvalidJson" },
  ARR_CUSTOM_FORMAT_FORMATS_EMPTY: { key: "settings.arrImportRefusalEmpty" },
  ARR_CUSTOM_FORMAT_FORMATS_TOO_MANY: {
    key: "settings.arrImportRefusalTooMany",
    fact: "maxFormats",
  },
};

/**
 * What to show for a failed import request: the translated reason when the
 * server named one this client knows, otherwise the server's own sentence, and
 * `fallback` when the failure carries no usable sentence.
 */
export function arrImportErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  const refusal =
    reason !== null && Object.hasOwn(ARR_IMPORT_REFUSAL_KEYS, reason)
      ? ARR_IMPORT_REFUSAL_KEYS[reason]!
      : null;
  if (!refusal) return userFacingGraphQlErrorMessage(error, fallback, t);
  if (!refusal.fact) return t(refusal.key);
  const value = graphQlErrorExtension(error, "VALIDATION_ERROR", refusal.fact);
  return value === null
    ? userFacingGraphQlErrorMessage(error, fallback, t)
    : t(refusal.key, { [refusal.fact]: value });
}
