import { useEffect, useMemo, useState } from "react";
import { useQuery } from "urql";

import { DashboardPanelLoad } from "@/components/views/dashboard-view";
import { DashboardPanel, DashboardPanelEmpty } from "./dashboard-panel";
import { DASHBOARD_PANEL_ICONS } from "./dashboard-panel-icons";
import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import {
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { dashboardLifecycleUpcomingQuery } from "@/lib/graphql/queries";
import { useRuleOutputDescriptors } from "@/lib/hooks/use-rule-builder-schema";
import type { DashboardTier } from "@/lib/utils/dashboard-layout";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  lifecycleUpcomingActionHeads,
  lifecycleUpcomingGroupHead,
  lifecycleUpcomingPanelSettings,
  lifecycleUpcomingTiming,
} from "@/lib/utils/lifecycle-upcoming-panel";
import { buildRulesPath } from "@/lib/utils/routing";
import {
  isRuleOutputDestructive,
  ruleOutputHeadLabel,
} from "@/lib/utils/rule-output-vocabulary";

type UpcomingCandidate = {
  id: string;
  ruleName: string;
  subjectKind: string;
  subjectLabel: string;
  titleName: string;
  state: string;
  actions: { heads: string[] } | null;
  dueAt: string;
  heldSince: string | null;
};

type UpcomingPreview = {
  lifecycleInstanceGates: { enabled: boolean; actionsPaused: boolean };
  lifecycleCandidates: UpcomingCandidate[];
};

/** Candidates still on their way to an action. */
const UPCOMING_STATES = ["OBSERVING", "PENDING_ACTION", "DUE"];
/** Only rules that act; a dry run never does. */
const ACTING_RULE_STATES = ["ENABLED"];
const UPCOMING_LIMIT = 25;
/** More selected groups than this read as a count instead of pills. */
const MAX_GROUP_PILLS = 3;

/**
 * One placement of the upcoming lifecycle actions panel: the actions enabled
 * rules are about to run, soonest first, narrowed to the groups its settings
 * name. Mounted only for a placed panel, so default dashboards read nothing.
 */
export function LifecycleUpcomingPanel({
  panelId,
  settings: savedSettings,
  tier,
}: {
  panelId: string;
  settings: unknown;
  tier: DashboardTier;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const descriptors = useRuleOutputDescriptors("lifecycle");
  const settings = useMemo(
    () => lifecycleUpcomingPanelSettings(savedSettings),
    [savedSettings],
  );
  const actions = useMemo(
    () => lifecycleUpcomingActionHeads(settings.actions),
    [settings.actions],
  );
  const variables = useMemo(
    () => ({
      states: UPCOMING_STATES,
      ruleStates: ACTING_RULE_STATES,
      actions,
      limit: UPCOMING_LIMIT,
    }),
    [actions],
  );
  const [now, setNow] = useState(() => Date.now());
  const [{ data, error, fetching }, refresh] = useQuery<UpcomingPreview>({
    query: dashboardLifecycleUpcomingQuery,
    variables,
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

  const gates = data?.lifecycleInstanceGates;
  const candidates = data?.lifecycleCandidates ?? [];
  const headLabel = (head: string) => ruleOutputHeadLabel(t, descriptors, head);
  const title = settings.name || t("dashboard.lifecycleUpcoming");
  const shownGroups = settings.actions.slice(0, MAX_GROUP_PILLS);
  const hiddenGroups = settings.actions.length - shownGroups.length;
  const idPrefix = selectorId("dashboard-lifecycle-upcoming", panelId);

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
        icon={DASHBOARD_PANEL_ICONS["lifecycle-upcoming"]}
        title={title}
        pills={
          <>
            {shownGroups.map((group) => (
              <Badge
                key={group}
                id={selectorId(idPrefix, "group", group)}
                className="max-w-[10rem]"
              >
                <span className="truncate">
                  {headLabel(lifecycleUpcomingGroupHead(group))}
                </span>
              </Badge>
            ))}
            {hiddenGroups > 0 ? (
              <Badge
                id={selectorId(idPrefix, "more-groups")}
                className="tabular-nums"
              >
                +{hiddenGroups}
              </Badge>
            ) : null}
            {gates?.enabled && gates.actionsPaused ? (
              <Badge id={selectorId(idPrefix, "paused")} tone="warning">
                {t("dashboard.lifecycleUpcomingPaused")}
              </Badge>
            ) : null}
          </>
        }
        linkTo={buildRulesPath("lifecycle", "candidates")}
        linkLabel={t("settings.lifecycleRules")}
        bodyClassName="overflow-y-auto"
      >
        {gates && !gates.enabled ? (
          <DashboardPanelEmpty message={t("dashboard.lifecycleUpcomingOff")} />
        ) : candidates.length === 0 ? (
          <DashboardPanelEmpty
            message={t(
              settings.actions.length > 0
                ? "dashboard.lifecycleUpcomingEmptyFiltered"
                : "dashboard.lifecycleUpcomingEmpty",
            )}
          />
        ) : (
          <ul id={selectorId(idPrefix, "list")}>
            {candidates.map((candidate) => {
              const timing = lifecycleUpcomingTiming(candidate, now);
              const exactDue = formatUiDateTime(
                candidate.dueAt,
                dateTimeFormat,
                {
                  fallback: candidate.dueAt,
                },
              );
              const held = candidate.heldSince
                ? t("settings.lifecycleCandidatesHeldSince", {
                    time: formatUiDateTime(
                      candidate.heldSince,
                      dateTimeFormat,
                      {
                        fallback: candidate.heldSince,
                      },
                    ),
                  })
                : null;
              const subject =
                candidate.subjectKind !== "title"
                  ? `${candidate.titleName} · ${candidate.subjectLabel}`
                  : candidate.titleName;
              return (
                <li
                  key={candidate.id}
                  id={selectorId(idPrefix, "row", candidate.id)}
                  className="min-w-0 space-y-1 border-b border-border px-3 py-2 last:border-b-0"
                  title={tier === "compact" && held ? held : undefined}
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      className="min-w-0 flex-1 truncate text-[12px] font-medium text-[var(--scry-ink2)]"
                      title={subject}
                    >
                      {subject}
                    </span>
                    <span className="flex min-w-0 max-w-[45%] items-center gap-1.5 overflow-hidden whitespace-nowrap text-[11px]">
                      {(candidate.actions?.heads ?? []).map((head) => (
                        <span
                          key={head}
                          data-lifecycle-action-head={head}
                          className={
                            isRuleOutputDestructive(descriptors, head)
                              ? "font-medium text-[var(--scry-warning-text)]"
                              : "text-[var(--scry-muted)]"
                          }
                        >
                          {headLabel(head)}
                        </span>
                      ))}
                    </span>
                    <time
                      className={
                        timing.due
                          ? "shrink-0 text-[11px] font-medium tabular-nums text-[var(--scry-warning-text)]"
                          : "shrink-0 text-[11px] tabular-nums text-[var(--scry-ink2)]"
                      }
                      dateTime={candidate.dueAt}
                      title={exactDue}
                    >
                      {timing.due
                        ? t("settings.lifecycleCountdownDueNow")
                        : timing.labelKey
                          ? t(timing.labelKey, timing.values)
                          : exactDue}
                    </time>
                  </div>
                  {tier !== "compact" ? (
                    <div className="flex min-w-0 gap-2 text-[10px] text-[var(--scry-muted2)]">
                      <span className="min-w-0 flex-1 truncate">
                        {candidate.ruleName}
                      </span>
                      {held ? (
                        <span className="shrink-0 text-[var(--scry-warning-text)]">
                          {held}
                        </span>
                      ) : null}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </DashboardPanel>
    </DashboardPanelLoad>
  );
}
