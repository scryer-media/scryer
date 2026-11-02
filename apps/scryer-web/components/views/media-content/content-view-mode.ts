export type ContentViewMode = "compact" | "poster-table" | "poster" | "graph";

export const CONTENT_VIEW_MODE_STORAGE_KEY = "scryer:content-view-mode";

function contentViewModeStorageKey(scope?: string | null) {
  const normalizedScope = scope?.trim();
  return normalizedScope
    ? `${CONTENT_VIEW_MODE_STORAGE_KEY}:${normalizedScope}`
    : CONTENT_VIEW_MODE_STORAGE_KEY;
}

function parseStoredContentViewMode(
  value: string | null,
): ContentViewMode | null {
  switch (value) {
    case "compact":
      return "compact";
    case "poster":
      return "poster";
    case "graph":
      return "graph";
    case "poster-table":
    case "table":
    case "list":
      return "poster-table";
    default:
      return null;
  }
}

export function readStoredContentViewMode(
  scope?: string | null,
): ContentViewMode {
  if (typeof window === "undefined") {
    return "poster";
  }

  try {
    const scoped = parseStoredContentViewMode(
      window.localStorage.getItem(contentViewModeStorageKey(scope)),
    );
    if (scoped) {
      return scoped;
    }
    return (
      parseStoredContentViewMode(
        window.localStorage.getItem(CONTENT_VIEW_MODE_STORAGE_KEY),
      ) ?? "poster"
    );
  } catch {
    return "poster";
  }
}

export function writeStoredContentViewMode(
  mode: ContentViewMode,
  scope?: string | null,
) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(contentViewModeStorageKey(scope), mode);
  } catch {
    // Ignore persistence failures.
  }
}
