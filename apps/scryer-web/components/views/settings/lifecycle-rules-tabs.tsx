import { History, ListChecks, Wrench } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { UnderlineTabLink } from "@/components/common/underline-filter-button";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import { buildRulesPath } from "@/lib/utils/routing";
import type { LifecycleRulesSection } from "@/components/root/types";

const LIFECYCLE_RULES_SECTIONS: {
  section: LifecycleRulesSection;
  labelKey: string;
  icon: LucideIcon;
}[] = [
  { section: "rules", labelKey: "settings.lifecycleNavRules", icon: Wrench },
  {
    section: "candidates",
    labelKey: "settings.lifecycleCandidatesTitle",
    icon: ListChecks,
  },
  {
    section: "history",
    labelKey: "settings.lifecycleNavHistory",
    icon: History,
  },
];

/// The Lifecycle Rules page's panes, as route tabs right above the pane they
/// open. The rule list is the default tab, so a bare lifecycle-rules link
/// opens it. The rule list adds its side-panel buttons at the strip's right
/// end.
export function LifecycleRulesTabs({
  activeSection,
  actions,
}: {
  activeSection: LifecycleRulesSection;
  /// Buttons for the open pane, at the right end of the tab strip.
  actions?: ReactNode;
}) {
  const t = useTranslate();
  return (
    <div className="flex flex-wrap items-end gap-3 border-b border-[var(--scry-border3)]">
      <div
        id="settings-lifecycle-rules-tabs"
        role="tablist"
        aria-label={t("settings.lifecycleRules")}
        className="relative top-px flex min-h-12 flex-wrap gap-x-2"
      >
        {LIFECYCLE_RULES_SECTIONS.map((item) => (
          <UnderlineTabLink
            key={item.section}
            id={selectorId("settings-lifecycle-rules-tab", item.section)}
            to={buildRulesPath("lifecycle", item.section)}
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
