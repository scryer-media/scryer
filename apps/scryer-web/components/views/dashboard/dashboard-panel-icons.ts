import {
  ActivitySquare,
  Database,
  Download,
  FolderInput,
  Gauge,
  HardDrive,
  Inbox,
  type LucideIcon,
} from "lucide-react";

import { LifecycleRulesIcon } from "@/components/common/lifecycle-rules-icon";
import type { DashboardPanelKind } from "@/lib/utils/dashboard-layout";

/**
 * One icon per panel kind, shared by the panel header and the edit tray so the
 * two can never show different icons for the same panel.
 */
export const DASHBOARD_PANEL_ICONS: Readonly<
  Record<DashboardPanelKind, LucideIcon>
> = {
  stats: Gauge,
  requests: Inbox,
  "manual-imports": FolderInput,
  "recently-imported": Download,
  indexers: Database,
  "download-clients": Download,
  storage: HardDrive,
  "active-queue": ActivitySquare,
  "recycle-bin": Inbox,
  "lifecycle-upcoming": LifecycleRulesIcon,
};
