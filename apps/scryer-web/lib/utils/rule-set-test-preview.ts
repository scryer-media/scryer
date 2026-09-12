import type { RuleSetDraft } from "@/lib/types/rule-sets";

export type RuleSetTestSelection = {
  titleId: string | null;
  episodeId: string | null;
  releaseName: string;
  sizeGib: string;
};

export type RuleSetTestMutationInput = {
  titleId: string;
  episodeId?: string;
  releaseName: string;
  sizeBytes?: number;
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
}: {
  draft: RuleSetDraft | null;
  editRuleSetId: string | null;
  copySourceRuleSetId: string | null;
  testRuleSetId: string | null;
  titleId: string;
  episodeId?: string;
  releaseName: string;
  sizeBytes?: number;
}): RuleSetTestMutationInput {
  const selection = { titleId, episodeId, releaseName, sizeBytes };
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

export function ruleSetTestFingerprint(
  draft: RuleSetDraft | null,
  selection: RuleSetTestSelection,
  editRuleSetId: string | null,
  copySourceRuleSetId: string | null,
  testRuleSetId: string | null,
): string {
  return JSON.stringify({ draft, selection, editRuleSetId, copySourceRuleSetId, testRuleSetId });
}

export function canTestRuleSet(
  selection: RuleSetTestSelection,
  requiresEpisode: boolean,
): boolean {
  return Boolean(
    selection.titleId &&
      selection.releaseName.trim() &&
      (!requiresEpisode || selection.episodeId),
  );
}

export function isCurrentRuleSetTest(request: number, currentRequest: number): boolean {
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

export function sizeBytesFromGib(value: string):
  | { value: undefined }
  | { value: number }
  | { error: string } {
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
