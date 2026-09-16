import * as React from "react";
import {
  ClipboardList,
  Eye,
  EyeOff,
  Edit,
  RefreshCw,
  Search,
  Trash2,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { LoadingMark } from "@/components/common/loading-mark";

type ActionButtonProps = {
  id: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  active?: boolean;
  loading?: boolean;
  destructive?: boolean;
  disabled?: boolean;
  onClick?: () => void;
};

type Props = {
  monitored: boolean;
  searchMonitoredLabel?: string;
  monitoredUpdating?: boolean;
  searchMonitoredLoading?: boolean;
  interactiveSearchLoading?: boolean;
  refreshAndScanLoading?: boolean;
  deleteLoading?: boolean;
  onToggleMonitoring?: () => void;
  onSearchMonitored?: () => void;
  onInteractiveSearch?: () => void;
  onRefreshAndScan?: () => void;
  onRequestDelete?: () => void;
  onHistory?: () => void;
  searchNotice?: React.ReactNode;
  settingsPanel?: React.ReactNode;
  interactiveSearchPanel?: React.ReactNode;
};

function ActionButton({
  id,
  label,
  icon: Icon,
  active = false,
  loading = false,
  destructive = false,
  disabled = false,
  onClick,
}: ActionButtonProps) {
  return (
    <Button
      id={id}
      type="button"
      variant="ghost"
      className={cn(
        "h-[84px] rounded-none border-0 bg-card/85 px-3 py-3 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
        "flex flex-col items-center justify-center gap-2",
        active && "bg-accent/45 text-foreground",
        destructive && "text-destructive hover:bg-destructive/10 hover:text-destructive",
      )}
      disabled={disabled || loading}
      onClick={onClick}
    >
      {loading ? (
        <LoadingMark className="size-6" />
      ) : (
        <Icon className="size-6" />
      )}
      <span className="text-center text-[11px] font-semibold uppercase tracking-[0.14em]">
        {label}
      </span>
    </Button>
  );
}

export function OverviewControlPanel({
  monitored,
  searchMonitoredLabel,
  monitoredUpdating = false,
  searchMonitoredLoading = false,
  interactiveSearchLoading = false,
  refreshAndScanLoading = false,
  deleteLoading = false,
  onToggleMonitoring,
  onSearchMonitored,
  onInteractiveSearch,
  onRefreshAndScan,
  onRequestDelete,
  onHistory,
  searchNotice,
  settingsPanel,
  interactiveSearchPanel,
}: Props) {
  const t = useTranslate();
  const [expandedPanel, setExpandedPanel] = React.useState<"settings" | "interactive" | null>(null);
  const hasInteractiveSearch = Boolean(interactiveSearchPanel);
  const showPersistentSearchNotice = Boolean(searchNotice) && expandedPanel !== "interactive";
  const resolvedSearchMonitoredLabel =
    searchMonitoredLabel ?? t("label.search");

  const handleToggleSettings = React.useCallback(() => {
    setExpandedPanel((current) => (current === "settings" ? null : "settings"));
  }, []);

  const handleToggleInteractiveSearch = React.useCallback(() => {
    setExpandedPanel((current) => {
      const next = current === "interactive" ? null : "interactive";
      if (next === "interactive") {
        onInteractiveSearch?.();
      }
      return next;
    });
  }, [onInteractiveSearch]);

  return (
    <Card id="title-overview-control-panel" className="overflow-hidden p-0">
      <CardContent className="space-y-0 p-0">
        <div
          className={cn(
            "grid grid-cols-2 gap-px bg-border/70 sm:grid-cols-3",
            hasInteractiveSearch ? "lg:grid-cols-7" : "lg:grid-cols-6",
          )}
        >
          <ActionButton
            id="title-overview-toggle-monitoring"
            label={monitored ? t("title.unmonitorAction") : t("title.monitorAction")}
            icon={monitored ? EyeOff : Eye}
            active={monitored}
            destructive={monitored}
            loading={monitoredUpdating}
            disabled={!onToggleMonitoring}
            onClick={onToggleMonitoring}
          />
          <ActionButton
            id="title-overview-search-monitored"
            label={resolvedSearchMonitoredLabel}
            icon={Zap}
            loading={searchMonitoredLoading}
            disabled={!onSearchMonitored}
            onClick={onSearchMonitored}
          />
          {hasInteractiveSearch ? (
            <ActionButton
              id="title-overview-interactive-search"
              label={t("label.interactive")}
              icon={Search}
              active={expandedPanel === "interactive"}
              loading={interactiveSearchLoading}
              disabled={!onInteractiveSearch}
              onClick={handleToggleInteractiveSearch}
            />
          ) : null}
          <ActionButton
            id="title-overview-refresh-and-scan"
            label={t("label.refresh")}
            icon={RefreshCw}
            loading={refreshAndScanLoading}
            disabled={!onRefreshAndScan}
            onClick={onRefreshAndScan}
          />
          <ActionButton
            id="title-overview-history"
            label={t("activity.history")}
            icon={ClipboardList}
            disabled={!onHistory}
            onClick={onHistory}
          />
          <ActionButton
            id="title-overview-edit-settings"
            label={t("label.edit")}
            icon={Edit}
            active={expandedPanel === "settings"}
            disabled={!settingsPanel}
            onClick={handleToggleSettings}
          />
          <ActionButton
            id="title-overview-delete"
            label={t("label.delete")}
            icon={Trash2}
            destructive
            loading={deleteLoading}
            disabled={!onRequestDelete}
            onClick={onRequestDelete}
          />
        </div>

        {showPersistentSearchNotice ? (
          <div id="title-overview-search-notice" className="border-t border-border bg-card/70 p-4">
            {searchNotice}
          </div>
        ) : null}

        {expandedPanel === "settings" && settingsPanel ? (
          <div id="title-overview-settings-panel" className="border-t border-border bg-card/70">
            {settingsPanel}
          </div>
        ) : null}

        {expandedPanel === "interactive" && interactiveSearchPanel ? (
          <div id="title-overview-interactive-search-panel" className="border-t border-border bg-card/70">
            {interactiveSearchPanel}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
