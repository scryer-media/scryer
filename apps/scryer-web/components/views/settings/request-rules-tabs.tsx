import { FlaskConical, ListChecks, Wrench } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { buildRulesPath } from "@/lib/utils/routing";
import type { RequestRulesSection } from "@/components/root/types";
import {
  RulePaneTabs,
  type RulePaneTab,
} from "@/components/views/settings/rule-pane-tabs";

const REQUEST_RULES_TABS: readonly RulePaneTab<RequestRulesSection>[] = [
  {
    section: "rules",
    labelKey: "settings.requestNavRules",
    icon: Wrench,
    to: buildRulesPath("request", "rules"),
  },
  {
    section: "try",
    labelKey: "settings.requestNavTry",
    icon: FlaskConical,
    to: buildRulesPath("request", "try"),
  },
  {
    section: "decisions",
    labelKey: "settings.requestNavDecisions",
    icon: ListChecks,
    to: buildRulesPath("request", "decisions"),
  },
];

export function RequestRulesTabs({
  activeSection,
  actions,
}: {
  activeSection: RequestRulesSection;
  actions?: ReactNode;
}) {
  const t = useTranslate();
  return (
    <RulePaneTabs
      id="settings-request-rules-tabs"
      tabIdPrefix="settings-request-rules-tab"
      ariaLabel={t("settings.requestRules")}
      tabs={REQUEST_RULES_TABS}
      activeSection={activeSection}
      actions={actions}
    />
  );
}
