type NamedTab = { title: string; scryerTitle?: string };

// Update only label metadata on GraphiQL's existing tab objects. Persist with
// storeTabs, never moveTab: moving tabs also reloads the Monaco editor models.
export function renameExplorerTab<T extends NamedTab>(tabs: T[], index: number, name: string): T[] | null {
  const title = name.trim().slice(0, 100);
  const tab = tabs[index];
  if (!tab || !title) return null;
  tab.scryerTitle = title;
  tab.title = title;
  return [...tabs];
}

export function restoreExplorerTabNames<T extends NamedTab>(tabs: T[]): T[] | null {
  let changed = false;
  for (const tab of tabs) {
    // GraphiQL derives titles again when editing; explicit names take precedence.
    if (typeof tab.scryerTitle === "string" && tab.scryerTitle && tab.title !== tab.scryerTitle) {
      tab.title = tab.scryerTitle;
      changed = true;
    }
  }
  return changed ? [...tabs] : null;
}
