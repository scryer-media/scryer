import type * as React from "react";
import { RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { InfoHelp } from "@/components/common/info-help";
import { LoadingMark } from "@/components/common/loading-mark";
import { useTranslate } from "@/lib/context/translate-context";

// The pieces every rules pane is built from, so the Lifecycle and Request
// Rules pages read like the Lists and History pages: a heading with its
// count and actions, a toolbar band for filters, and tables and forms in the
// same rounded shells.

/// The same heading the Lists page puts over each of its sections.
export const SECTION_HEADING =
  "font-display text-[17px] font-bold text-[var(--scry-ink)]";

/// Tables sit in the same rounded shell as the Lists tables.
export const TABLE_SHELL = "rounded-[12px] border border-[var(--scry-border3)]";

/// The filter and toolbar band above a table, as on the History page.
export const TOOLBAR_SHELL =
  "flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3 md:flex-row md:flex-wrap md:items-end";

export const TOOLBAR_SELECT_LABEL = "text-xs text-[var(--scry-muted)]";

export function PaneSection({
  id,
  title,
  help,
  count,
  actions,
  children,
}: {
  id: string;
  title: string;
  help?: string;
  count?: number;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="space-y-3">
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <h2 className={SECTION_HEADING}>{title}</h2>
          {count !== undefined ? (
            <Badge id={`${id}-count`} tone="neutral">
              {count}
            </Badge>
          ) : null}
          {help ? <InfoHelp text={help} ariaLabel={title} /> : null}
        </div>
        {actions ? (
          <div className="flex flex-wrap items-center gap-2">{actions}</div>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function PaneError({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <p
      role="alert"
      className="whitespace-pre-wrap rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-4 py-3 text-[13px] text-[var(--scry-danger-text)]"
    >
      {message}
    </p>
  );
}

/// What a section shows in place of its table: loading, or why it is empty.
export function PaneEmpty({
  id,
  loading,
  children,
}: {
  id: string;
  loading: boolean;
  children: React.ReactNode;
}) {
  const t = useTranslate();
  return (
    <div
      id={id}
      className="flex items-center justify-center gap-2 rounded-[12px] border border-dashed border-[var(--scry-border3)] px-4 py-8 text-center text-[13px] text-[var(--scry-muted)]"
    >
      {loading ? (
        <>
          <LoadingMark className="h-4 w-4" />
          {t("label.loading")}
        </>
      ) : (
        children
      )}
    </div>
  );
}

export function RefreshButton({
  id,
  loading,
  onClick,
}: {
  id: string;
  loading: boolean;
  onClick: () => void;
}) {
  const t = useTranslate();
  return (
    <Button
      id={id}
      type="button"
      variant="secondary"
      size="sm"
      disabled={loading}
      onClick={onClick}
    >
      {loading ? (
        <LoadingMark className="h-4 w-4" />
      ) : (
        <RefreshCw className="h-4 w-4" />
      )}
      {t("label.refresh")}
    </Button>
  );
}

/// The surface a pane's own form sits on, such as trying a rule against a
/// request or a title: the toolbar band's look, laid out as a block.
export const FORM_SHELL =
  "space-y-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-4";
