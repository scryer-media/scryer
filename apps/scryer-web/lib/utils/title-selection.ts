import type { TitleRecord } from "@/lib/types/titles";

// Catalog refreshes hand React a brand new titles array on every poll, even
// when nothing about the selection changed. Effects that fetch previews for the
// selection must depend on the selected ids themselves, not on that array's
// identity, or a background scan restarts them on every refresh.
const SELECTED_TITLE_IDS_SEPARATOR = "\n";

// A stable key for the SET of selected title ids: order and object identity do
// not affect it, so it only changes when the selection really changes.
export function selectedTitleIdsKey(
  titles: readonly Pick<TitleRecord, "id">[],
): string {
  if (titles.length === 0) {
    return "";
  }
  const ids = new Set<string>();
  for (const title of titles) {
    ids.add(title.id);
  }
  return [...ids].sort().join(SELECTED_TITLE_IDS_SEPARATOR);
}
