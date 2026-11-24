import { FlaskConical, History, ListChecks } from "lucide-react";
import { LifecycleRulesIcon } from "@/components/common/lifecycle-rules-icon";
import type { ReactNode } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { buildRulesPath } from "@/lib/utils/routing";
import type { LifecycleRulesSection } from "@/components/root/types";
import {
  RulePaneTabs,
  type RulePaneTab,
} from "@/components/views/settings/rule-pane-tabs";

const LIFECYCLE_RULES_TABS: readonly RulePaneTab<LifecycleRulesSection>[] = [
  {
    section: "rules",
    labelKey: "settings.lifecycleNavRules",
    icon: LifecycleRulesIcon,
    to: buildRulesPath("lifecycle", "rules"),
  },
  {
    section: "try",
    labelKey: "settings.lifecycleNavTry",
    icon: FlaskConical,
    to: buildRulesPath("lifecycle", "try"),
  },
  {
    section: "candidates",
    labelKey: "settings.lifecycleCandidatesTitle",
    icon: ListChecks,
    to: buildRulesPath("lifecycle", "candidates"),
  },
  {
    section: "history",
    labelKey: "settings.lifecycleNavHistory",
    icon: History,
    to: buildRulesPath("lifecycle", "history"),
  },
];

export function LifecycleRulesTabs({
  activeSection,
  actions,
}: {
  activeSection: LifecycleRulesSection;
  actions?: ReactNode;
}) {
  const t = useTranslate();
  return (
    <RulePaneTabs
      id="settings-lifecycle-rules-tabs"
      tabIdPrefix="settings-lifecycle-rules-tab"
      ariaLabel={t("settings.lifecycleRules")}
      tabs={LIFECYCLE_RULES_TABS}
      activeSection={activeSection}
      actions={actions}
    />
  );
}
