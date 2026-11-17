import * as React from "react";
import { X } from "lucide-react";

import { IconButton } from "@/components/ui/icon-button";
import { useTranslate } from "@/lib/context/translate-context";
import { GutterPanelContext } from "@/lib/context/gutter-panel-context";
import { useGutterPanel } from "@/lib/hooks/use-gutter-panel";

/// Template cards and reference tables need more room than a plugin list, so
/// rule side panels are wider than the Lists one and open over the content on
/// most screens.
const RULE_SIDE_PANEL_SIZE = { width: 560, gap: 32 };

/// The side panels of a rule page (starter templates, input reference) that
/// slide out from the right edge of the settings scroll pane, like the Lists
/// plugins panel. They open only on a click, at any screen size, and only one
/// is open at a time. `rootRef` goes on the page's outermost element and
/// `columnRef` on its centered content column; while `active` is false the
/// panels are closed and not measured.
export function useRuleSidePanels<Panel extends string>(active = true) {
  const rootRef = React.useRef<HTMLDivElement>(null);
  const scrollPaneRef = React.useRef<HTMLElement | null>(null);
  const columnRef = React.useRef<HTMLDivElement>(null);
  // Runs before the gutter hook's measurement, which needs the scroll pane.
  React.useLayoutEffect(() => {
    scrollPaneRef.current =
      rootRef.current?.closest<HTMLElement>(
        '[data-slot="settings-main-scroll"]',
      ) ?? null;
  });
  const { frame } = useGutterPanel(
    scrollPaneRef,
    columnRef,
    active ? RULE_SIDE_PANEL_SIZE : null,
  );
  const [openPanel, setOpenPanel] = React.useState<Panel | null>(null);
  if (!active && openPanel !== null) setOpenPanel(null);
  const buttonProps = (panel: Panel) => ({
    open: openPanel === panel,
    onOpenChange: (open: boolean) => setOpenPanel(open ? panel : null),
  });
  const panelProps = (panel: Panel) => ({ frame, ...buttonProps(panel) });
  const close = React.useCallback(() => setOpenPanel(null), []);
  return { rootRef, columnRef, buttonProps, panelProps, close };
}

/// The slim header a rule side panel keeps in view while its body scrolls
/// beneath it, with the panel's own close button.
export function RuleSidePanelHeader({
  icon,
  title,
}: {
  icon: React.ReactNode;
  title: React.ReactNode;
}) {
  const t = useTranslate();
  const gutterPanel = React.useContext(GutterPanelContext);
  return (
    <div className="sticky top-0 z-10 flex items-center justify-between gap-2 border-b border-[var(--scry-border3)] bg-[var(--scry-bg)] py-1.5 pl-3 pr-1.5">
      <h2 className="flex min-w-0 items-center gap-2 truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
        {icon}
        {title}
      </h2>
      {gutterPanel ? (
        <IconButton
          label={t("label.close")}
          appearance="ghost"
          className="size-7"
          onClick={gutterPanel.close}
        >
          <X className="h-4 w-4" />
        </IconButton>
      ) : null}
    </div>
  );
}

/// The row of side-panel buttons a rule page shows at the right of its
/// content column, above the rule table.
export function RuleSidePanelButtons({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      {children}
    </div>
  );
}
