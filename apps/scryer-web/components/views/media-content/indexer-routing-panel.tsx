import * as React from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Label } from "@/components/ui/label";
import { ChevronDown, ChevronUp, SlidersVertical } from "lucide-react";
import { IndexerCategoryPicker } from "./indexer-category-picker";
import type { ViewCategoryId } from "./indexer-category-picker";
import { getDefaultIndexerRouting } from "@/lib/constants/indexers";
import type {
  IndexerCategoryRoutingSettings,
  IndexerRecord,
} from "@/lib/types";
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
  tone: Extract<BoxedActionButtonTone, "reorder">;
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
    const configuredIds = activeScopeIndexerRoutingOrder.filter(
      (indexerId) => indexerById[indexerId],
    );
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
      <div className="mt-5 space-y-3">
        {orderedIndexerIds.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.indexerRoutingNoIndexers")}
          </p>
        ) : (
          orderedIndexerIds.map((indexerId, index) => {
            const indexer = indexerById[indexerId];
            if (!indexer) {
              return null;
            }
            const routing =
              activeScopeIndexerRouting[indexer.id] ??
              getDefaultIndexerRouting(activeQualityScopeId);
            const controlsDisabled =
              indexerRoutingLoading || indexerRoutingSaving;
            const enabledId = selectorId(
              routing.enabled
                ? "indexer-routing-disable"
                : "indexer-routing-enable",
              indexer.name,
            );
            const categoriesId = selectorId(
              "indexer-routing-categories",
              indexer.name,
            );
            return (
              <div
                key={indexer.id}
                id={selectorId("indexer-routing-row", indexer.name)}
                data-ui="settings-table-row"
                className="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 sm:p-4"
              >
                <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                  <div className="flex items-center gap-1">
                    <span className="w-5 text-center text-sm tabular-nums text-muted-foreground">
                      {index + 1}
                    </span>
                    <IndexerRoutingActionButton
                      id={selectorId("indexer-routing-move-up", indexer.name)}
                      tone="reorder"
                      label={`${t("label.moveUp")} ${indexer.name}`}
                      onClick={() => onMoveUp(indexer.id)}
                      disabled={controlsDisabled || index === 0}
                    >
                      <ChevronUp className="h-4 w-4" />
                    </IndexerRoutingActionButton>
                    <IndexerRoutingActionButton
                      id={selectorId("indexer-routing-move-down", indexer.name)}
                      tone="reorder"
                      label={`${t("label.moveDown")} ${indexer.name}`}
                      onClick={() => onMoveDown(indexer.id)}
                      disabled={
                        controlsDisabled ||
                        index >= orderedIndexerIds.length - 1
                      }
                    >
                      <ChevronDown className="h-4 w-4" />
                    </IndexerRoutingActionButton>
                  </div>
                  <div className="order-first flex w-full min-w-0 items-center gap-2 @lg/facet-page:order-none @lg/facet-page:w-auto @lg/facet-page:flex-1">
                    <span className="truncate font-semibold text-[var(--scry-ink2)]">
                      {indexer.name}
                    </span>
                    {!indexer.isEnabled ? (
                      <span
                        className="shrink-0 rounded-full border border-[var(--scry-border2)] px-2 py-0.5 text-xs text-muted-foreground"
                        title={`${t("settings.indexerRoutingGloballyEnabled")}: ${t("label.no")}`}
                      >
                        {t("label.disabled")}
                      </span>
                    ) : null}
                  </div>
                  <label
                    htmlFor={enabledId}
                    className="ml-auto flex cursor-pointer items-center gap-2 text-sm font-medium"
                  >
                    {t("settings.indexerRoutingEnabled")}
                    <Checkbox
                      id={enabledId}
                      size="large"
                      checked={indexer.isEnabled && routing.enabled}
                      disabled={controlsDisabled || !indexer.isEnabled}
                      onCheckedChange={(checked) =>
                        onEnabledChange(indexer.id, checked === true)
                      }
                    />
                  </label>
                </div>
                <div className="mt-3 min-w-0 space-y-1.5">
                  <Label
                    htmlFor={categoriesId}
                    className="text-xs text-muted-foreground"
                  >
                    {t("settings.indexerRoutingCategories")}
                  </Label>
                  <IndexerCategoryPicker
                    triggerId={categoriesId}
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
                    capsCategories={indexer.capsCategories}
                    disabled={controlsDisabled}
                    categoriesLabel={`${t("settings.indexerRoutingCategories")} (${indexer.name})`}
                    onChange={(categories) =>
                      onCategoriesChange(indexer.id, categories)
                    }
                  />
                </div>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
});
