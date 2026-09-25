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

/** Retention bounds the server accepts for recycled items, in days. */
export const RECYCLE_BIN_MIN_RETENTION_DAYS = 1;
export const RECYCLE_BIN_MAX_RETENTION_DAYS = 3650;

/** Fields to change; an omitted field keeps the stored value on the server. */
export type RecycleBinSettingsChanges = {
  enabled?: boolean;
  path?: string | null;
  retentionDays?: number;
};

export type RecycleBinSettingsInput = {
  enabled?: boolean;
  path?: string | null;
  retentionDays?: number;
};

/**
 * Parses the retention field. Returns null for anything the server would
 * reject, so the form can refuse to save it.
 */
export function parseRecycleBinRetentionDays(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const days = Number(trimmed);
  return days >= RECYCLE_BIN_MIN_RETENTION_DAYS && days <= RECYCLE_BIN_MAX_RETENTION_DAYS
    ? days
    : null;
}

/**
 * Builds a partial update input carrying only the fields being changed, so a
 * toggle never overwrites the stored path or retention. A blank path is sent
 * as null, which restores the default `.scryer-recycle` folder under each
 * library root.
 */
export function buildRecycleBinSettingsInput(
  changes: RecycleBinSettingsChanges,
): RecycleBinSettingsInput {
  const input: RecycleBinSettingsInput = {};
  if (changes.enabled !== undefined) input.enabled = changes.enabled;
  if (changes.path !== undefined) {
    const trimmedPath = changes.path?.trim() ?? "";
    input.path = trimmedPath === "" ? null : trimmedPath;
  }
  if (changes.retentionDays !== undefined) input.retentionDays = changes.retentionDays;
  return input;
}
