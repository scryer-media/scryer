import * as React from "react";
import {
  Bell,
  BellOff,
  Edit,
  Loader2,
  RefreshCw,
  Search,
  Trash2,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

type ActionButtonProps = {
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
  settingsPanel?: React.ReactNode;
  interactiveSearchPanel?: React.ReactNode;
};

function ActionButton({
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
      type="button"
      variant="ghost"
      className={cn(
        "h-[84px] rounded-none border-0 bg-card/85 px-3 py-3 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
        "flex flex-col items-center justify-center gap-2",
        active && "bg-accent/45 text-foreground",
        destructive && "hover:bg-destructive/10 hover:text-destructive",
      )}
      disabled={disabled || loading}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="size-8 animate-spin" />
      ) : (
        <Icon className="size-8" />
      )}
      <span className="text-center text-[11px] font-semibold uppercase tracking-[0.14em]">
        {label}
      </span>
    </Button>
  );
}

export function OverviewControlPanel({
  monitored,
  searchMonitoredLabel = "Search Monitored",
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
  settingsPanel,
  interactiveSearchPanel,
}: Props) {
  const [expandedPanel, setExpandedPanel] = React.useState<"settings" | "interactive" | null>(null);
  const hasInteractiveSearch = Boolean(interactiveSearchPanel);

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
    <Card className="overflow-hidden p-0">
      <CardContent className="space-y-0 p-0">
        <div
          className={cn(
            "grid grid-cols-2 gap-px bg-border/70 sm:grid-cols-3",
            hasInteractiveSearch ? "lg:grid-cols-6" : "lg:grid-cols-5",
          )}
        >
          <ActionButton
            label="Monitor"
            icon={monitored ? BellOff : Bell}
            active={monitored}
            loading={monitoredUpdating}
            disabled={!onToggleMonitoring}
            onClick={onToggleMonitoring}
          />
          <ActionButton
            label={searchMonitoredLabel}
            icon={Zap}
            loading={searchMonitoredLoading}
            disabled={!onSearchMonitored}
            onClick={onSearchMonitored}
          />
          {hasInteractiveSearch ? (
            <ActionButton
              label="Interactive Search"
              icon={Search}
              active={expandedPanel === "interactive"}
              loading={interactiveSearchLoading}
              disabled={!onInteractiveSearch}
              onClick={handleToggleInteractiveSearch}
            />
          ) : null}
          <ActionButton
            label="Refresh & Scan"
            icon={RefreshCw}
            loading={refreshAndScanLoading}
            disabled={!onRefreshAndScan}
            onClick={onRefreshAndScan}
          />
          <ActionButton
            label="Edit"
            icon={Edit}
            active={expandedPanel === "settings"}
            disabled={!settingsPanel}
            onClick={handleToggleSettings}
          />
          <ActionButton
            label="Delete"
            icon={Trash2}
            destructive
            loading={deleteLoading}
            disabled={!onRequestDelete}
            onClick={onRequestDelete}
          />
        </div>

        {expandedPanel === "settings" && settingsPanel ? (
          <div className="border-t border-border bg-card/70">
            {settingsPanel}
          </div>
        ) : null}

        {expandedPanel === "interactive" && interactiveSearchPanel ? (
          <div className="border-t border-border bg-card/70">
            {interactiveSearchPanel}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
