export type RecycleBinFilterItem = {
  id: string;
  fileName: string;
  titleId: string | null;
  titleName: string | null;
  libraryId: string;
  libraryName: string;
  recycledAt: string;
};

export type RecycleBinGroup<TItem extends RecycleBinFilterItem> = {
  id: string;
  titleName: string;
  libraryName: string;
  items: TItem[];
};

/**
 * Groups recycle-bin entries by title for display. A title match keeps its
 * complete group visible; otherwise the text filter narrows individual files.
 */
export function groupRecycleBinItems<TItem extends RecycleBinFilterItem>(
  items: TItem[],
  filter: string,
  unassociatedTitleName: string,
): RecycleBinGroup<TItem>[] {
  const normalizedFilter = filter.trim().toLocaleLowerCase();
  const byTitle = new Map<string, RecycleBinGroup<TItem>>();
  for (const item of items) {
    const titleName = item.titleName?.trim() || unassociatedTitleName;
    const groupId = item.titleId ? `title:${item.titleId}` : `unassociated:${item.libraryId}`;
    const group = byTitle.get(groupId) ?? {
      id: groupId,
      titleName,
      libraryName: item.libraryName,
      items: [],
    };
    group.items.push(item);
    byTitle.set(groupId, group);
  }

  return Array.from(byTitle.values())
    .map((group) => {
      const titleMatches =
        normalizedFilter.length === 0 || group.titleName.toLocaleLowerCase().includes(normalizedFilter);
      const visibleItems = titleMatches
        ? group.items
        : group.items.filter((item) => item.fileName.toLocaleLowerCase().includes(normalizedFilter));
      return {
        ...group,
        items: [...visibleItems].sort((a, b) => b.recycledAt.localeCompare(a.recycledAt)),
      };
    })
    .filter((group) => group.items.length > 0)
    .sort((a, b) => b.items[0].recycledAt.localeCompare(a.items[0].recycledAt));
}
