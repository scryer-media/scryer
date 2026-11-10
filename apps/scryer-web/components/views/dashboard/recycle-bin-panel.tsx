import { useEffect, useState } from "react";
import { useQuery } from "urql";
import { Link } from "react-router";

import { DashboardPanelLoad } from "@/components/views/dashboard-view";
import { DashboardPanel, DashboardPanelEmpty } from "./dashboard-panel";
import { DASHBOARD_PANEL_ICONS } from "./dashboard-panel-icons";
import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import {
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { dashboardRecycleBinQuery } from "@/lib/graphql/queries";
import { formatBytes } from "@/lib/utils/activity-utils";
import { buildRecycleBinItemHref } from "@/lib/utils/recycle-bin";
import type { DashboardTier } from "@/lib/utils/dashboard-layout";

type RecycleBinPreview = {
  recycledItems: {
    totalCount: number;
    items: Array<{
      id: string;
      fileName: string;
      sizeBytes: number;
      titleName: string | null;
      libraryName: string;
      scheduledDeletionAt: string;
    }>;
  };
};

/** Mounted only for a placed panel, so default dashboards do not scan recycle bins. */
export function RecycleBinPanel({ tier }: { tier: DashboardTier }) {
  const t = useTranslate();
  const [now, setNow] = useState(() => Date.now());
  const [{ data, error, fetching }, refresh] = useQuery<RecycleBinPreview>({
    query: dashboardRecycleBinQuery,
    requestPolicy: "network-only",
  });

  useEffect(() => {
    const handlePulse = (event: Event) => {
      if (!(event instanceof CustomEvent) || document.hidden) return;
      const source = (event as CustomEvent<NavigationBadgesRefreshDetail>)
        .detail?.source;
      if (source === "poll" || source === "focus") {
        setNow(Date.now());
        refresh({ requestPolicy: "network-only" });
      }
    };
    window.addEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
    return () =>
      window.removeEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
  }, [refresh]);

  return (
    <DashboardPanelLoad
      state={{
        loading: fetching,
        ready: Boolean(data),
        error: error
          ? userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t)
          : null,
      }}
    >
      <DashboardPanel
        icon={DASHBOARD_PANEL_ICONS["recycle-bin"]}
        title={t("dashboard.recycleBin")}
        count={data?.recycledItems.totalCount}
        pills={<Badge>{t("dashboard.recycleBinWindow")}</Badge>}
        linkTo="/system/recycle-bin"
        linkLabel={t("settings.recycleBin")}
        bodyClassName="overflow-y-auto"
      >
        {!data?.recycledItems.items.length ? (
          <DashboardPanelEmpty message={t("dashboard.recycleBinEmpty")} />
        ) : (
          <ul>
            {data.recycledItems.items.map((item) => (
              <li
                key={item.id}
                className="min-w-0 space-y-1 border-b border-border px-3 py-2 last:border-b-0"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <Link
                    to={buildRecycleBinItemHref(item.id)}
                    className="min-w-0 flex-1 truncate text-[12px] font-medium underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    title={item.fileName}
                  >
                    {item.fileName}
                  </Link>
                  <time
                    className="shrink-0 text-[11px] tabular-nums text-[var(--scry-warning-text)]"
                    dateTime={item.scheduledDeletionAt}
                    title={new Date(item.scheduledDeletionAt).toLocaleString()}
                  >
                    {Date.parse(item.scheduledDeletionAt) <= now
                      ? t("dashboard.recycleBinDueNow")
                      : t("dashboard.recycleBinDueIn", {
                          hours: Math.ceil(
                            (Date.parse(item.scheduledDeletionAt) - now) /
                              3_600_000,
                          ),
                        })}
                  </time>
                </div>
                {tier !== "compact" ? (
                  <div className="flex min-w-0 gap-2 text-[10px] text-[var(--scry-muted2)]">
                    <span className="min-w-0 flex-1 truncate">
                      {item.libraryName}
                      {item.titleName ? ` · ${item.titleName}` : ""}
                    </span>
                    <span className="shrink-0 tabular-nums">
                      {formatBytes(item.sizeBytes)}
                    </span>
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </DashboardPanel>
    </DashboardPanelLoad>
  );
}
