import type * as React from "react";

import {
  ActiveQueuePanel,
  DashboardPanelLoad,
  DownloadClientsPanel,
  IndexersPanel,
  ManualImportsPanel,
  RecentlyImportedPanel,
  RequestsPanel,
  StatsPanel,
  StoragePanel,
  type DashboardPanelData,
} from "@/components/views/dashboard-view";
import { combinePanelStates } from "@/lib/utils/dashboard-refresh";
import type {
  DashboardBreakpoint,
  DashboardPanelId,
  DashboardTier,
} from "@/lib/utils/dashboard-layout";

type DashboardPanelRenderArgs = {
  data: DashboardPanelData;
  tier: DashboardTier;
  breakpoint: DashboardBreakpoint;
};

/**
 * The renderer half of the panel registry. Sizes, tiers and default placement
 * live as data in `lib/utils/dashboard-layout.ts` (and are unit tested there);
 * icons live in `dashboard-panel-icons.ts`. Each renderer wraps its panel in
 * the load state the panel's data depends on.
 */
export const DASHBOARD_PANEL_RENDERERS: Readonly<
  Record<DashboardPanelId, (args: DashboardPanelRenderArgs) => React.ReactNode>
> = {
  stats: ({ data, tier, breakpoint }) => (
    <DashboardPanelLoad state={data.panels.overview}>
      <StatsPanel
        overview={data.overview}
        tier={tier}
        breakpoint={breakpoint}
      />
    </DashboardPanelLoad>
  ),
  requests: ({ data, tier }) => (
    <DashboardPanelLoad state={data.panels.requests}>
      <RequestsPanel
        tier={tier}
        requests={data.requests}
        totalCount={data.overview?.pendingRequestCount ?? data.requests.length}
        actionRequestId={data.actionRequestId}
        onApprove={data.onApproveRequest}
        onDismiss={data.onDismissRequest}
      />
    </DashboardPanelLoad>
  ),
  "manual-imports": ({ data, tier }) => (
    <DashboardPanelLoad state={data.panels.imports}>
      <ManualImportsPanel
        tier={tier}
        items={data.importActivity}
        totalCount={data.importActivityTotal}
        busyItemId={data.importActionItemId}
        onImport={data.onImportItem}
        onMarkFailed={data.onMarkImportFailed}
        onRemove={data.onRemoveImportItem}
      />
    </DashboardPanelLoad>
  ),
  "recently-imported": ({ data, tier }) => (
    <DashboardPanelLoad state={data.panels.recent}>
      <RecentlyImportedPanel items={data.recentImports} tier={tier} />
    </DashboardPanelLoad>
  ),
  indexers: ({ data }) => (
    <DashboardPanelLoad state={data.panels.overview}>
      <IndexersPanel overview={data.overview} />
    </DashboardPanelLoad>
  ),
  "download-clients": ({ data }) => (
    <DashboardPanelLoad
      state={combinePanelStates(data.panels.overview, data.panels.queue)}
    >
      <DownloadClientsPanel
        overview={data.overview}
        queueItems={data.queueItems}
      />
    </DashboardPanelLoad>
  ),
  storage: ({ data, tier }) => (
    <DashboardPanelLoad state={data.panels.storage}>
      <StoragePanel roots={data.storageRoots} tier={tier} />
    </DashboardPanelLoad>
  ),
  "active-queue": ({ data, tier }) => (
    <DashboardPanelLoad state={data.panels.queue}>
      <ActiveQueuePanel
        items={data.queueItems}
        totalCount={data.queueTotal}
        tier={tier}
      />
    </DashboardPanelLoad>
  ),
};
