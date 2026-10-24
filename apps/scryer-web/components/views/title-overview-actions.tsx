import * as React from "react";
import {
  ClipboardList,
  Edit,
  Eye,
  EyeOff,
  RefreshCw,
  Search,
  Trash2,
  Zap,
} from "lucide-react";

import {
  TitleWorkspaceActionButton,
  TitleWorkspaceActionGrid,
} from "@/components/views/media-content/title-workspace-primitives";
import { useTranslate } from "@/lib/context/translate-context";

export const TITLE_OVERVIEW_CONTROL_PANEL_ID = "title-overview-control-panel";
export const TITLE_OVERVIEW_SEARCH_MONITORED_ID = "title-overview-search-monitored";
export const TITLE_OVERVIEW_SETTINGS_PANEL_ID = "title-overview-settings-panel";

export type TitleOverviewInteractiveSearchAction = {
  open: boolean;
  loading: boolean;
  disabled?: boolean;
  /** The region the parent renders while the search is open. */
  panelId: string;
  onToggle: () => void;
};

/**
 * The action bar at the top of a title overview, shared by the movie and the
 * series/anime overviews so both offer the same actions in the same order with
 * the same ids. Render it with `key={title.id}` so the settings panel closes
 * when the overview moves to another title.
 */
export function TitleOverviewActions({
  monitored,
  monitoredUpdating,
  onToggleMonitoring,
  searchButtonId = TITLE_OVERVIEW_SEARCH_MONITORED_ID,
  searchLoading,
  onSearch,
  searchNotice,
  interactiveSearch,
  refreshLoading,
  onRefresh,
  onHistory,
  settingsPanel,
  deleteLoading,
  onDelete,
  busy = false,
}: {
  monitored: boolean;
  monitoredUpdating: boolean;
  onToggleMonitoring?: () => void;
  searchButtonId?: string;
  searchLoading: boolean;
  onSearch: () => void;
  /** Shown below the actions, e.g. when searching needs a download client. */
  searchNotice?: React.ReactNode;
  interactiveSearch?: TitleOverviewInteractiveSearchAction;
  refreshLoading: boolean;
  onRefresh: () => void;
  onHistory: () => void;
  settingsPanel?: React.ReactNode;
  deleteLoading: boolean;
  onDelete?: () => void;
  /** Another action on this title is running; hold the rest until it ends. */
  busy?: boolean;
}) {
  const t = useTranslate();
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const interactiveOpen = interactiveSearch?.open === true;

  return (
    <>
      <TitleWorkspaceActionGrid
        id={TITLE_OVERVIEW_CONTROL_PANEL_ID}
        columns={interactiveSearch ? 7 : 6}
      >
        <TitleWorkspaceActionButton
          id="title-overview-toggle-monitoring"
          icon={monitored ? EyeOff : Eye}
          label={monitored ? t("title.unmonitorAction") : t("title.monitorAction")}
          active={monitored}
          pressed={monitored}
          loading={monitoredUpdating}
          disabled={busy || !onToggleMonitoring}
          onClick={() => onToggleMonitoring?.()}
        />
        <TitleWorkspaceActionButton
          id={searchButtonId}
          icon={Zap}
          label={t("label.search")}
          loading={searchLoading}
          disabled={busy}
          onClick={onSearch}
        />
        {interactiveSearch ? (
          <TitleWorkspaceActionButton
            id="title-overview-interactive-search"
            icon={Search}
            label={t("label.interactive")}
            active={interactiveOpen}
            loading={interactiveOpen && interactiveSearch.loading}
            disabled={interactiveSearch.disabled}
            expanded={interactiveOpen}
            controlsId={interactiveSearch.panelId}
            onClick={interactiveSearch.onToggle}
          />
        ) : null}
        <TitleWorkspaceActionButton
          id="title-overview-refresh-and-scan"
          icon={RefreshCw}
          label={t("label.refresh")}
          loading={refreshLoading}
          disabled={busy}
          onClick={onRefresh}
        />
        <TitleWorkspaceActionButton
          id="title-overview-history"
          icon={ClipboardList}
          label={t("activity.history")}
          disabled={busy}
          onClick={onHistory}
        />
        <TitleWorkspaceActionButton
          id="title-overview-edit-settings"
          icon={Edit}
          label={t("label.edit")}
          active={settingsOpen}
          disabled={busy || !settingsPanel}
          expanded={settingsOpen}
          controlsId={TITLE_OVERVIEW_SETTINGS_PANEL_ID}
          onClick={() => setSettingsOpen((current) => !current)}
        />
        <TitleWorkspaceActionButton
          id="title-overview-delete"
          icon={Trash2}
          label={t("label.delete")}
          destructive
          loading={deleteLoading}
          disabled={busy || !onDelete}
          onClick={() => onDelete?.()}
        />
      </TitleWorkspaceActionGrid>

      {searchNotice && !interactiveOpen ? (
        <div id="title-overview-search-notice" className="mb-3">
          {searchNotice}
        </div>
      ) : null}

      {settingsOpen && settingsPanel ? (
        <div
          id={TITLE_OVERVIEW_SETTINGS_PANEL_ID}
          role="region"
          aria-label={t("label.edit")}
          className="mb-3 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]"
        >
          {settingsPanel}
        </div>
      ) : null}
    </>
  );
}
