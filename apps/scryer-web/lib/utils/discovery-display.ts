const SOURCE_ID_PATTERN = /^[a-z][a-z0-9_+-]*:[a-z0-9_+-]+:/i;
const NUMERIC_ID_PATTERN = /^\d+$/;

type DiscoveryDisplayItem = {
  displayTitle: string;
  originalTitle: string | null;
  sortTitle?: string | null;
  targetKey: string;
};

function hasNonLatinTitleCharacters(value: string) {
  for (const character of value) {
    if (character.codePointAt(0)! > 0x024f) {
      return true;
    }
  }
  return false;
}

export function usefulDiscoveryTitle(value: string | null | undefined) {
  const title = value?.trim() ?? "";
  if (!title) {
    return null;
  }
  if (SOURCE_ID_PATTERN.test(title) || NUMERIC_ID_PATTERN.test(title)) {
    return null;
  }
  return title;
}

export function discoveryItemDisplayTitle(item: DiscoveryDisplayItem) {
  const displayTitle = usefulDiscoveryTitle(item.displayTitle);
  const sortTitle = usefulDiscoveryTitle(item.sortTitle);
  const alternateTitle = usefulDiscoveryTitle(item.originalTitle);
  if (
    alternateTitle &&
    displayTitle &&
    hasNonLatinTitleCharacters(displayTitle)
  ) {
    return alternateTitle;
  }
  return displayTitle || sortTitle || alternateTitle || item.targetKey;
}
