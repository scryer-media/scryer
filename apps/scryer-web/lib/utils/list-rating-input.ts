/** Preserve intermediate decimal input while filters carry numeric minimums. */
export function parseListRatingInput(
  text: string,
): { text: string; value: number | null } | null {
  if (!/^\d*(?:\.\d*)?$/.test(text)) return null;
  const value = text === "" || text === "." ? null : Number(text);
  return value === null || Number.isFinite(value) ? { text, value } : null;
}

export function listRatingInputText(
  draft: string | undefined,
  value: number | null,
): string {
  if (draft !== undefined && parseListRatingInput(draft)?.value === value)
    return draft;
  return value === null ? "" : String(value);
}
