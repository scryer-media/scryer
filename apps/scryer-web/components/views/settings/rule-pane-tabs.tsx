import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { UnderlineTabLink } from "@/components/common/underline-filter-button";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";

export type RulePaneTab<Section extends string> = {
  section: Section;
  labelKey: string;
  icon: LucideIcon;
  to: string;
};

/// The tab strip across the top of a rules page whose panes are routes. Page
/// actions, such as the starter rules and input reference buttons, sit at the
/// strip's right end.
export function RulePaneTabs<Section extends string>({
  id,
  tabIdPrefix,
  ariaLabel,
  tabs,
  activeSection,
  actions,
}: {
  id: string;
  tabIdPrefix: string;
  ariaLabel: string;
  tabs: readonly RulePaneTab<Section>[];
  activeSection: Section;
  actions?: ReactNode;
}) {
  const t = useTranslate();
  return (
    <div className="flex flex-wrap items-end gap-3 border-b border-[var(--scry-border3)]">
      <div
        id={id}
        role="tablist"
        aria-label={ariaLabel}
        className="relative top-px flex min-h-12 flex-wrap gap-x-2"
      >
        {tabs.map((item) => (
          <UnderlineTabLink
            key={item.section}
            id={selectorId(tabIdPrefix, item.section)}
            to={item.to}
            selected={activeSection === item.section}
            icon={<item.icon className="h-4 w-4" />}
            label={t(item.labelKey)}
          />
        ))}
      </div>
      {actions ? (
        <div className="ml-auto flex flex-wrap items-center gap-2 pb-2">
          {actions}
        </div>
      ) : null}
    </div>
  );
}
