import type { TitleRecord } from "../types/titles.ts";
import {
  itemMatchKeys,
  type TraversalItem,
} from "./library-graph-traversal.ts";

/** The readable catalog title selected directly or reached through discovery. */
export function graphSelectionCatalogTitle(
  nodeTitle: TitleRecord | undefined,
  item: TraversalItem | null,
  catalogIndex: ReadonlyMap<string, TitleRecord>,
  titles: readonly TitleRecord[],
): TitleRecord | null {
  if (nodeTitle) return nodeTitle;
  if (!item) return null;
  const resolved = item.resolvedTitleId?.trim();
  const byId = resolved
    ? titles.find((title) => title.id === resolved)
    : undefined;
  if (byId) return byId;
  for (const key of itemMatchKeys(item)) {
    const title = catalogIndex.get(key);
    if (title) return title;
  }
  return null;
}
