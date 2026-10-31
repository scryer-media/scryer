import type { LucideIcon } from "lucide-react";
import { Database, KeyRound, Search, Upload } from "lucide-react";
import type { IndexerSettingsTab } from "@/components/root/types";

/// The Indexers page's tabs as both the page tabs and the sidebar subnav draw
/// them, so the two always show the same label and icon.
export const INDEXER_SETTINGS_TAB_ITEMS: Record<
  IndexerSettingsTab,
  { labelKey: string; icon: LucideIcon; beta: boolean }
> = {
  indexers: { labelKey: "settings.indexers", icon: Database, beta: false },
  search: { labelKey: "settings.indexerSearch", icon: Search, beta: true },
  clients: { labelKey: "settings.indexerClients", icon: KeyRound, beta: true },
  seedingProfiles: {
    labelKey: "settings.seedingProfiles",
    icon: Upload,
    beta: false,
  },
};
