import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { FileCode2, Power } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { RuleSetRecord } from "@/lib/types/rule-sets";

type RulesRoutingPanelProps = {
  facet: string;
  ruleSets: RuleSetRecord[];
  loading: boolean;
  saving: boolean;
  onToggleFacet: (ruleSetId: string, enabled: boolean) => void;
};

export const RulesRoutingPanel = React.memo(function RulesRoutingPanel({
  facet,
  ruleSets,
  loading,
  saving,
  onToggleFacet,
}: RulesRoutingPanelProps) {
  const t = useTranslate();
  // Only show globally-enabled rules
  const enabledRules = React.useMemo(
    () => ruleSets.filter((r) => r.enabled),
    [ruleSets],
  );

  if (loading) return null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <FileCode2 className="h-4 w-4" />
          {t("settings.rulesFacetSection", { facet })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {enabledRules.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.ruleNoFacetRules")}
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.ruleName")}</TableHead>
                <TableHead>{t("settings.ruleDescription")}</TableHead>
                <TableHead className="text-center">{t("settings.rulePriority")}</TableHead>
                <TableHead className="text-center">{t("settings.ruleAppliedFacets")}</TableHead>
                <TableHead className="text-right">{t("settings.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {enabledRules.map((rule) => {
                const isGlobal = rule.appliedFacets.length === 0;
                const isEnabledForFacet = isGlobal || rule.appliedFacets.includes(facet);

                return (
                  <TableRow key={rule.id}>
                    <TableCell className="font-medium">{rule.name}</TableCell>
                    <TableCell className="max-w-[200px] truncate text-muted-foreground">
                      {rule.description || "—"}
                    </TableCell>
                    <TableCell className="text-center">{rule.priority}</TableCell>
                    <TableCell className="text-center">
                      {isGlobal ? (
                        <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
                          {t("settings.ruleGlobal")}
                        </span>
                      ) : isEnabledForFacet ? (
                        <span className="rounded bg-emerald-900/40 px-1.5 py-0.5 text-xs text-emerald-300">
                          {t("label.enabled")}
                        </span>
                      ) : (
                        <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
                          {t("label.disabled")}
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      {isGlobal ? (
                        <span className="text-xs text-muted-foreground italic">
                          {t("settings.ruleGlobal")}
                        </span>
                      ) : (
                        <Button
                          size="sm"
                          variant="secondary"
                          disabled={saving}
                          onClick={() => onToggleFacet(rule.id, !isEnabledForFacet)}
                          className={
                            isEnabledForFacet
                              ? "border-red-700/70 bg-red-900/60 text-red-200 hover:bg-red-900/80 hover:text-red-100"
                              : "border-emerald-300/70 dark:border-emerald-700/70 bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-100 hover:bg-emerald-200 dark:hover:bg-emerald-800/80"
                          }
                        >
                          <Power className="mr-1 h-3.5 w-3.5" />
                          {isEnabledForFacet ? t("label.disabled") : t("label.enabled")}
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
});
