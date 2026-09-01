import * as React from "react";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { IconButton } from "@/components/ui/icon-button";
import { ChevronDown, ChevronUp, Power, PowerOff, SlidersVertical } from "lucide-react";
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
import { selectorId } from "@/lib/utils/dom-ids";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";

type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function IndexerRoutingActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "enabled" | "disabled" | "reorder">;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
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
    <section
      id="indexer-routing-panel"
      className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6"
    >
      <div className="flex items-center gap-2.5">
        <SlidersVertical className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">
          {t("settings.indexerRoutingScope", {
            scope: scopeLabel,
          })}
        </h2>
      </div>
      <div className="mt-5">
        <div className="overflow-x-auto rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
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
                    <TableRow
                      key={indexer.id}
                      id={selectorId("indexer-routing-row", indexer.name)}
                      data-ui="settings-table-row"
                    >
                      <TableCell>{index + 1}</TableCell>
                      <TableCell>{indexer.name}</TableCell>
                      <TableCell className="w-[30rem] min-w-[30rem] max-w-[30rem]">
                        <IndexerCategoryPicker
                          triggerId={selectorId(
                            "indexer-routing-categories",
                            indexer.name,
                          )}
                          panelId={selectorId(
                            "indexer-routing-categories-panel",
                            indexer.name,
                          )}
                          categoryIdPrefix={selectorId(
                            "indexer-routing-category",
                            indexer.name,
                          )}
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
                            id={selectorId(
                              routing.enabled
                                ? "indexer-routing-disable"
                                : "indexer-routing-enable",
                              indexer.name,
                            )}
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
                            id={selectorId("indexer-routing-move-up", indexer.name)}
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
                            id={selectorId("indexer-routing-move-down", indexer.name)}
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
      </div>
    </section>
  );
});
