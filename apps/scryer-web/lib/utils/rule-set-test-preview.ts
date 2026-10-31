import type { RuleSetDraft } from "@/lib/types/rule-sets";

export type RuleSetTestMode = "release" | "storedFile";

/** Listing-fact form fields, as typed. Empty strings mean "unknown". */
export type RuleSetTestListingDraft = {
  protocol: string;
  publishedAt: string;
  thumbsUp: string;
  thumbsDown: string;
  /** "", "true" or "false". */
  isPasswordProtected: string;
  /** Comma-separated language codes. */
  indexerLanguages: string;
  /** A JSON object. */
  extra: string;
};

export const EMPTY_RULE_SET_TEST_LISTING: RuleSetTestListingDraft = {
  protocol: "",
  publishedAt: "",
  thumbsUp: "",
  thumbsDown: "",
  isPasswordProtected: "",
  indexerLanguages: "",
  extra: "",
};

export type RuleSetTestSelection = {
  titleId: string | null;
  episodeId: string | null;
  releaseName: string;
  sizeGib: string;
  mode?: RuleSetTestMode;
  mediaFileId?: string | null;
  listing?: RuleSetTestListingDraft;
};

/** `TestRuleSetInput.listing`. Omitted facts are unknown to rules. */
export type RuleSetTestListingInput = {
  protocol?: "usenet" | "torrent" | "http";
  publishedAt?: string;
  thumbsUp?: number;
  thumbsDown?: number;
  isPasswordProtected?: boolean;
  indexerLanguages?: string[];
  extra?: Record<string, unknown>;
};

/** `TestRuleSetPayload.listing`: the listing facts the rules read. */
export type RuleSetTestListingFacts = {
  protocol?: "usenet" | "torrent" | "http" | null;
  publishedAt?: string | null;
  ageDays?: number | null;
  thumbsUp?: number | null;
  thumbsDown?: number | null;
  isPasswordProtected?: boolean | null;
  indexerLanguages?: string[];
  extra?: Record<string, unknown> | null;
  capturedAt?: string;
};

export type RuleSetTestMutationInput = {
  titleId: string;
  episodeId?: string;
  releaseName?: string;
  sizeBytes?: number;
  listing?: RuleSetTestListingInput;
  mediaFileId?: string;
  draft?: RuleSetDraft;
  editRuleSetId?: string;
  copySourceRuleSetId?: string;
  copyDisablesSource?: boolean;
  testRuleSetId?: string;
};

export function buildRuleSetTestInput({
  draft,
  editRuleSetId,
  copySourceRuleSetId,
  testRuleSetId,
  titleId,
  episodeId,
  releaseName,
  sizeBytes,
  listing,
  mediaFileId,
}: {
  draft: RuleSetDraft | null;
  editRuleSetId: string | null;
  copySourceRuleSetId: string | null;
  testRuleSetId: string | null;
  titleId: string;
  episodeId?: string;
  releaseName?: string;
  sizeBytes?: number;
  listing?: RuleSetTestListingInput;
  mediaFileId?: string;
}): RuleSetTestMutationInput {
  // A stored file is scored from its own size and frozen listing facts.
  const selection = mediaFileId
    ? { titleId, episodeId, mediaFileId }
    : {
        titleId,
        episodeId,
        releaseName,
        sizeBytes,
        ...(listing ? { listing } : {}),
      };
  return testRuleSetId
    ? { ...selection, testRuleSetId }
    : {
        ...selection,
        draft: draft!,
        editRuleSetId: editRuleSetId || undefined,
        copySourceRuleSetId: copySourceRuleSetId || undefined,
        copyDisablesSource: Boolean(copySourceRuleSetId),
      };
}

// RFC 3339 with seconds and a zone, as the server's parser requires.
const RFC3339 =
  /^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;
// RFC 2822, the form a newznab `pubDate` takes.
const RFC2822 =
  /^(?:[A-Za-z]{3},\s*)?\d{1,2}\s+[A-Za-z]{3}\s+\d{4}\s+\d{2}:\d{2}(?::\d{2})?\s+(?:[+-]\d{4}|[A-Za-z]{1,5})$/;
const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Turn the listing-fact form into mutation input. Errors are i18n keys.
 * The publish time is kept as written, like a live listing's; only a date
 * without a time is completed, to midnight UTC.
 */
export function listingInputFromDraft(
  draft: RuleSetTestListingDraft,
): { value: RuleSetTestListingInput | undefined } | { error: string } {
  const value: RuleSetTestListingInput = {};
  if (
    draft.protocol === "usenet" ||
    draft.protocol === "torrent" ||
    draft.protocol === "http"
  ) {
    value.protocol = draft.protocol;
  }
  const publishedAt = draft.publishedAt.trim();
  if (publishedAt) {
    const normalized = DATE_ONLY.test(publishedAt)
      ? `${publishedAt}T00:00:00Z`
      : publishedAt;
    if (
      !(RFC3339.test(normalized) || RFC2822.test(normalized)) ||
      !Number.isFinite(Date.parse(normalized))
    ) {
      return { error: "settings.ruleTestListingPublishedAtInvalid" };
    }
    value.publishedAt = normalized;
  }
  for (const key of ["thumbsUp", "thumbsDown"] as const) {
    const raw = draft[key].trim();
    if (!raw) continue;
    const count = Number(raw);
    if (!Number.isInteger(count) || count < 0 || count > 2_147_483_647) {
      return { error: "settings.ruleTestListingVotesInvalid" };
    }
    value[key] = count;
  }
  if (draft.isPasswordProtected === "true") value.isPasswordProtected = true;
  if (draft.isPasswordProtected === "false") value.isPasswordProtected = false;
  const languages = draft.indexerLanguages
    .split(",")
    .map((language) => language.trim())
    .filter(Boolean);
  if (languages.length) value.indexerLanguages = languages;
  const extra = draft.extra.trim();
  if (extra) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(extra);
    } catch {
      return { error: "settings.ruleTestListingExtraInvalid" };
    }
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      Array.isArray(parsed)
    ) {
      return { error: "settings.ruleTestListingExtraInvalid" };
    }
    value.extra = parsed as Record<string, unknown>;
  }
  return { value: Object.keys(value).length ? value : undefined };
}

/** One row of the title's file list; a multi-episode file has one per episode. */
export type RuleSetTestStoredFileRow = {
  id: string;
  episodeId?: string | null;
  filePath?: string | null;
  grabbedReleaseTitle?: string | null;
};

export type RuleSetTestStoredFileOption = {
  id: string;
  label: string;
  episodeIds: string[];
};

/**
 * The stored files a tester can pick: one per file, labelled by the release
 * it was grabbed as (else its file name). With an episode selected, only the
 * files that cover it.
 */
export function storedFileOptions(
  rows: readonly RuleSetTestStoredFileRow[],
  episodeId: string | null,
): RuleSetTestStoredFileOption[] {
  const byId = new Map<string, RuleSetTestStoredFileOption>();
  for (const row of rows) {
    let option = byId.get(row.id);
    if (!option) {
      const fileName = (row.filePath ?? "").split(/[\\/]/).pop();
      option = {
        id: row.id,
        label: row.grabbedReleaseTitle || fileName || row.id,
        episodeIds: [],
      };
      byId.set(row.id, option);
    }
    if (row.episodeId && !option.episodeIds.includes(row.episodeId)) {
      option.episodeIds.push(row.episodeId);
    }
  }
  const options = [...byId.values()];
  return episodeId
    ? options.filter((option) => option.episodeIds.includes(episodeId))
    : options;
}

export function ruleSetTestFingerprint(
  draft: RuleSetDraft | null,
  selection: RuleSetTestSelection,
  editRuleSetId: string | null,
  copySourceRuleSetId: string | null,
  testRuleSetId: string | null,
): string {
  return JSON.stringify({
    draft,
    selection,
    editRuleSetId,
    copySourceRuleSetId,
    testRuleSetId,
  });
}

export function canTestRuleSet(
  selection: RuleSetTestSelection,
  requiresEpisode: boolean,
): boolean {
  if (!selection.titleId) return false;
  // A stored file brings its own episode when none is picked.
  if (selection.mode === "storedFile") return Boolean(selection.mediaFileId);
  return Boolean(
    selection.releaseName.trim() && (!requiresEpisode || selection.episodeId),
  );
}

export function isCurrentRuleSetTest(
  request: number,
  currentRequest: number,
): boolean {
  return request === currentRequest;
}

export class RuleSetTestRequestController {
  private request = 0;
  private busy = false;
  private disposed = false;

  activate(): void {
    this.disposed = false;
    this.busy = false;
    this.request += 1;
  }

  begin(): number | null {
    if (this.busy || this.disposed) return null;
    this.busy = true;
    this.request += 1;
    return this.request;
  }

  finish(request: number): boolean {
    if (!this.isCurrent(request)) return false;
    this.busy = false;
    return true;
  }

  dispose(): void {
    this.disposed = true;
    this.busy = false;
    this.request += 1;
  }

  isCurrent(request: number): boolean {
    return !this.disposed && isCurrentRuleSetTest(request, this.request);
  }
}

export function sizeBytesFromGib(
  value: string,
): { value: undefined } | { value: number } | { error: string } {
  if (value.trim() === "") return { value: undefined };
  const gib = Number(value);
  if (!Number.isFinite(gib) || gib < 0) {
    return { error: "Size must be a non-negative number of GiB." };
  }
  const bytes = Math.round(gib * 1024 ** 3);
  if (!Number.isSafeInteger(bytes)) {
    return { error: "Size is too large to preview safely." };
  }
  return { value: bytes };
}

export function formatSignedScore(value: number): string {
  return `${value >= 0 ? "+" : ""}${value}`;
}

export function shouldApplyRuleSetTestResponse(
  request: number,
  currentRequest: number,
  requestFingerprint: string,
  committedFingerprint: string,
): boolean {
  return (
    isCurrentRuleSetTest(request, currentRequest) &&
    requestFingerprint === committedFingerprint
  );
}
