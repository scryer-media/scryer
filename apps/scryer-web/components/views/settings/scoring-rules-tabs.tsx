import { SlidersHorizontal, Users } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { buildRulesPath } from "@/lib/utils/routing";
import type { ScoringRulesSection } from "@/components/root/types";
import {
  RulePaneTabs,
  type RulePaneTab,
} from "@/components/views/settings/rule-pane-tabs";

const SCORING_RULES_TABS: readonly RulePaneTab<ScoringRulesSection>[] = [
  {
    section: "rules",
    labelKey: "settings.scoringNavRules",
    icon: SlidersHorizontal,
    to: buildRulesPath("scoring", "rules"),
  },
  {
    section: "community",
    labelKey: "settings.scoringNavCommunity",
    icon: Users,
    to: buildRulesPath("scoring", "community"),
  },
];

export function ScoringRulesTabs({
  activeSection,
  actions,
}: {
  activeSection: ScoringRulesSection;
  actions?: ReactNode;
}) {
  const t = useTranslate();
  return (
    <RulePaneTabs
      id="settings-scoring-rules-tabs"
      tabIdPrefix="settings-scoring-rules-tab"
      ariaLabel={t("settings.rules")}
      tabs={SCORING_RULES_TABS}
      activeSection={activeSection}
      actions={actions}
    />
  );
}
