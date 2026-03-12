import * as React from "react";
import { Button } from "@/components/ui/button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { ChevronDown, ChevronUp, Power, PowerOff } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { IndexerCategoryPicker } from "./indexer-category-picker";
import type { ViewCategoryId } from "./indexer-category-picker";
import { getDefaultIndexerRouting } from "@/lib/constants/indexers";
import type { IndexerCategoryRoutingSettings, IndexerRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function IndexerRoutingActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "enabled" | "disabled" | "reorder">;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

type IndexerRoutingPanelProps = {
  scopeLabel: string;
  activeQualityScopeId: ViewCategoryId;
  indexers: IndexerRecord[];
  activeScopeIndexerRouting: IndexerRoutingRecord;
  activeScopeIndexerRoutingOrder: string[];
  indexerRoutingLoading: boolean;
  indexerRoutingSaving: boolean;
  onEnabledChange: (indexerId: string, enabled: boolean) => void;
  onCategoriesChange: (indexerId: string, categories: string[]) => void;
  onMoveUp: (indexerId: string) => void;
  onMoveDown: (indexerId: string) => void;
};

export const IndexerRoutingPanel = React.memo(function IndexerRoutingPanel({
  scopeLabel,
  activeQualityScopeId,
  indexers,
  activeScopeIndexerRouting,
  activeScopeIndexerRoutingOrder,
  indexerRoutingLoading,
  indexerRoutingSaving,
  onEnabledChange,
  onCategoriesChange,
  onMoveUp,
  onMoveDown,
}: IndexerRoutingPanelProps) {
  const t = useTranslate();
  const indexerById = React.useMemo(
    () => Object.fromEntries(indexers.map((indexer) => [indexer.id, indexer])),
    [indexers],
  );

  const orderedIndexerIds = React.useMemo(() => {
    const configuredIds = activeScopeIndexerRoutingOrder.filter((indexerId) => indexerById[indexerId]);
    const configuredIdSet = new Set(configuredIds);
    const missingIds = indexers
      .map((indexer) => indexer.id)
      .filter((indexerId) => !configuredIdSet.has(indexerId));
    return [...configuredIds, ...missingIds];
  }, [activeScopeIndexerRoutingOrder, indexerById, indexers]);

  return (
    <div>
      <Card>
        <CardHeader>
          <CardTitle>
            {t("settings.indexerRoutingScope", {
              scope: scopeLabel,
            })}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="overflow-x-auto rounded border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("settings.indexerRoutingPriority")}</TableHead>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead>{t("settings.indexerRoutingCategories")}</TableHead>
                  <TableHead className="text-center">{t("settings.indexerRoutingGloballyEnabled")}</TableHead>
                  <TableHead className="text-center">{t("settings.indexerRoutingEnabled")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {orderedIndexerIds.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-muted-foreground">
                      {t("settings.indexerRoutingNoIndexers")}
                    </TableCell>
                  </TableRow>
                ) : (
                  orderedIndexerIds.map((indexerId, index) => {
                    const indexer = indexerById[indexerId];
                    if (!indexer) {
                      return null;
                    }
                    const routing = activeScopeIndexerRouting[indexer.id] ?? getDefaultIndexerRouting(activeQualityScopeId);
                    return (
                      <TableRow key={indexer.id}>
                        <TableCell>{index + 1}</TableCell>
                        <TableCell>{indexer.name}</TableCell>
                        <TableCell className="w-[30rem] min-w-[30rem] max-w-[30rem]">
                          <IndexerCategoryPicker
                            value={routing.categories}
                            scope={activeQualityScopeId}
                            disabled={indexerRoutingLoading}
                            categoriesLabel={`${t("settings.indexerRoutingCategories")} (${indexer.name})`}
                            onChange={(categories) =>
                              onCategoriesChange(indexer.id, categories)
                            }
                          />
                        </TableCell>
                        <TableCell className="text-center align-middle">
                          <RenderBooleanIcon
                            value={indexer.isEnabled}
                            label={`${t("settings.indexerRoutingGloballyEnabled")}: ${indexer.name}`}
                          />
                        </TableCell>
                        <TableCell className="text-center align-middle">
                          <RenderBooleanIcon
                            value={indexer.isEnabled && routing.enabled}
                            label={`${t("settings.indexerRoutingEnabled")}: ${indexer.name}`}
                          />
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex items-center justify-end gap-2">
                            <IndexerRoutingActionButton
                              tone={routing.enabled ? "disabled" : "enabled"}
                              label={
                                routing.enabled
                                  ? t("label.disable")
                                  : t("label.enable")
                              }
                              onClick={() =>
                                onEnabledChange(indexer.id, !routing.enabled)
                              }
                              disabled={indexerRoutingLoading || indexerRoutingSaving || !indexer.isEnabled}
                            >
                              {routing.enabled ? (
                                <PowerOff className="h-3.5 w-3.5" />
                              ) : (
                                <Power className="h-3.5 w-3.5" />
                              )}
                            </IndexerRoutingActionButton>
                            <IndexerRoutingActionButton
                              tone="reorder"
                              label={`${t("label.moveUp")} ${indexer.name}`}
                              onClick={() => onMoveUp(indexer.id)}
                              disabled={
                                indexerRoutingLoading ||
                                indexerRoutingSaving ||
                                index === 0
                              }
                            >
                              <ChevronUp className="h-4 w-4" />
                            </IndexerRoutingActionButton>
                            <IndexerRoutingActionButton
                              tone="reorder"
                              label={`${t("label.moveDown")} ${indexer.name}`}
                              onClick={() => onMoveDown(indexer.id)}
                              disabled={
                                indexerRoutingLoading ||
                                indexerRoutingSaving ||
                                index >= orderedIndexerIds.length - 1
                              }
                            >
                              <ChevronDown className="h-4 w-4" />
                            </IndexerRoutingActionButton>
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  })
                )}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </div>
  );
});
