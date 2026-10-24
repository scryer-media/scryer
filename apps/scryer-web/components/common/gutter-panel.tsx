import * as React from "react";
import { ChevronDown } from "lucide-react";

import { SCROLL_FADE_CONTROL_CLASS } from "@/components/common/horizontal-scroll-fade";
import { Button } from "@/components/ui/button";
import { GutterPanelContext } from "@/lib/context/gutter-panel-context";
import type { GutterPanelFrame } from "@/lib/hooks/use-gutter-panel";
import { cn } from "@/lib/utils";

type GutterPanelProps = {
  id: string;
  label: string;
  frame: GutterPanelFrame;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: React.ReactNode;
};

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * A full-height side panel pinned to the right edge of the page content.
 * Docked in the gutter it covers nothing, so it is not modal and the page
 * stays usable beside it. Opened over the content it is modal: a backdrop
 * dims the page and dismisses the panel on click, and the panel holds focus
 * until it closes. The page's `GutterPanelButton` opens and closes it. The
 * panel stays mounted so content can be portaled into it.
 */
export function GutterPanel({ id, label, frame, open, onOpenChange, children }: GutterPanelProps) {
  const close = React.useCallback(() => onOpenChange(false), [onOpenChange]);
  const context = React.useMemo(() => ({ close }), [close]);
  const panelRef = React.useRef<HTMLElement>(null);
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const overlaying = open && !frame.docked;
  const [hasMoreBelow, setHasMoreBelow] = React.useState(false);

  const measureOverflow = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    setHasMoreBelow(element.scrollHeight - element.clientHeight - element.scrollTop > 1);
  }, []);

  React.useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    measureOverflow();
    if (typeof ResizeObserver === "undefined") return;
    // Content portaled in from elsewhere grows its slot, so watch the
    // children as well as the panel itself.
    const resizeObserver = new ResizeObserver(measureOverflow);
    resizeObserver.observe(element);
    for (const child of Array.from(element.children)) resizeObserver.observe(child);
    return () => resizeObserver.disconnect();
  }, [children, measureOverflow]);

  const scrollByPage = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    element.scrollBy({ top: Math.max(element.clientHeight * 0.8, 240), behavior: "smooth" });
  }, []);

  React.useEffect(() => {
    // Docked, the panel covers nothing, so focus and Escape are left to the page.
    const panel = panelRef.current;
    if (!overlaying || !panel) return;
    const returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panel.focus({ preventScroll: true });
    const handleKeyDown = (event: KeyboardEvent) => {
      // A dialog opened from the panel, such as an uninstall confirmation,
      // handles its own keys.
      const target = event.target instanceof Element ? event.target : null;
      if (target && !panel.contains(target) && target.closest('[role="dialog"], [role="alertdialog"]')) return;
      if (event.key === "Escape") {
        close();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
        (element) => element.getClientRects().length > 0,
      );
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!first || !last) {
        event.preventDefault();
        panel.focus({ preventScroll: true });
      } else if (event.shiftKey ? active === first || !panel.contains(active) || active === panel : active === last || !panel.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      if (returnFocus?.isConnected && panel.contains(document.activeElement)) {
        returnFocus.focus({ preventScroll: true });
      }
    };
  }, [close, overlaying]);

  return (
    <>
      {frame.docked ? null : (
        <div
          aria-hidden="true"
          onClick={close}
          className={cn(
            "fixed inset-0 z-40 bg-black/50 transition-opacity duration-300",
            open ? "opacity-100" : "pointer-events-none opacity-0",
          )}
        />
      )}
      <aside
        ref={panelRef}
        id={id}
        aria-label={label}
        tabIndex={-1}
        style={{ top: frame.top, right: frame.right, width: frame.width }}
        className={cn(
          "fixed bottom-0 z-40 flex flex-col border-l border-[var(--scry-border)] bg-[var(--scry-bg)] text-[var(--scry-ink2)] outline-none duration-300 ease-in-out",
          // The panel sits on top of the page: a soft shadow when it only
          // takes gutter space, a deeper one when it covers the content.
          frame.docked
            ? "shadow-[-12px_0_32px_rgba(0,0,0,0.28)]"
            : "shadow-[-20px_0_50px_rgba(0,0,0,0.42)]",
          // Visible at once on open so it can take focus; hidden only once
          // it has slid away on close.
          open
            ? "visible translate-x-0 opacity-100 transition-[translate,opacity]"
            : "pointer-events-none invisible translate-x-full opacity-0 transition-[translate,opacity,visibility]",
        )}
      >
        <div
          ref={scrollRef}
          onScroll={measureOverflow}
          className="scry-scroll min-h-0 flex-1 overflow-y-auto"
        >
          <GutterPanelContext.Provider value={context}>{children}</GutterPanelContext.Provider>
        </div>
        {hasMoreBelow ? (
          <>
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-14 bg-gradient-to-b from-transparent to-[var(--scry-bg)]"
            />
            <button
              type="button"
              aria-label={`${label}: scroll down`}
              onClick={scrollByPage}
              className={cn(SCROLL_FADE_CONTROL_CLASS, "bottom-3 left-1/2 -translate-x-1/2")}
            >
              <ChevronDown className="size-7 stroke-[3]" aria-hidden="true" />
            </button>
          </>
        ) : null}
      </aside>
    </>
  );
}

type GutterPanelButtonProps = {
  panelId: string;
  label: string;
  icon: React.ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

/** Opens and closes a gutter panel from the page header: solid while the panel is closed, outlined while it is open. */
export function GutterPanelButton({ panelId, label, icon, open, onOpenChange }: GutterPanelButtonProps) {
  return (
    <Button
      type="button"
      variant={open ? "outline" : "primary"}
      className="h-10 shrink-0 rounded-[10px] px-3 text-[13px]"
      aria-controls={panelId}
      aria-expanded={open}
      onClick={() => onOpenChange(!open)}
    >
      {icon}
      {label}
    </Button>
  );
}
