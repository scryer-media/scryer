import * as React from "react";
import { useClient } from "urql";
import { Tag, X } from "lucide-react";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  updateSeriesMovieTagsMutation,
  updateTitleTagsMutation,
} from "@/lib/graphql/mutations";
import { useTitleTagDefinitions } from "@/lib/hooks/use-title-tag-definitions";
import type { TitleTagDefinition } from "@/lib/types/title-tags";
import {
  availableTitleTagLabels,
  isEmptyTitleTagsDelta,
  titleTagsDelta,
  userTitleTags,
} from "@/lib/utils/title-tags";

/// Sentinel for the select's resting state. Radix selects need a non-empty
/// value, and the control is an action ("add this one"), not a field with a
/// current value.
const ADD_PLACEHOLDER_VALUE = "__add_title_tag__";

export type TitleTagsPickerProps = {
  /** Labels currently applied. Reserved `scryer:` entries are ignored. */
  value: readonly string[] | null | undefined;
  onChange: (labels: string[]) => void;
  definitions: readonly TitleTagDefinition[];
  loading?: boolean;
  disabled?: boolean;
  idPrefix: string;
  /**
   * Labels to keep out of the add list even though the registry defines them
   * and this picker does not hold them — used by the bulk dialog so a label
   * cannot be queued for adding and removing at once.
   */
  excludedLabels?: readonly string[];
  /** Overrides the "no tags applied" line; the bulk pickers word it their own way. */
  emptyValueText?: string;
  /** Places applied tags beside the add selector for use in a settings table. */
  layout?: "stacked" | "horizontal" | "table";
};

/**
 * Chips plus a registry-backed select. There is deliberately no free-text
 * entry: an administrator decides which tags exist, and everything else only
 * decides which titles carry them.
 */
export function TitleTagsPicker({
  value,
  onChange,
  definitions,
  loading = false,
  disabled = false,
  idPrefix,
  excludedLabels,
  emptyValueText,
  layout = "stacked",
}: TitleTagsPickerProps) {
  const t = useTranslate();
  const applied = React.useMemo(() => userTitleTags(value), [value]);
  const excluded = React.useMemo(
    () => new Set(userTitleTags(excludedLabels)),
    [excludedLabels],
  );
  const options = React.useMemo(
    () =>
      availableTitleTagLabels(definitions, applied).filter(
        (label) => !excluded.has(label),
      ),
    [applied, definitions, excluded],
  );
  const registryIsEmpty = !loading && definitions.length === 0;

  const addLabel = React.useCallback(
    (label: string) => {
      if (label === ADD_PLACEHOLDER_VALUE) {
        return;
      }
      onChange(userTitleTags([...applied, label]));
    },
    [applied, onChange],
  );

  const removeLabel = React.useCallback(
    (label: string) => {
      onChange(applied.filter((candidate) => candidate !== label));
    },
    [applied, onChange],
  );

  const appliedTags = applied.length > 0 ? (
    <div className="flex flex-wrap gap-1.5">
      {applied.map((label) => (
        <span
          key={label}
          className="inline-flex max-w-full items-center gap-1.5 rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.15)] py-1 pl-2 pr-1.5 text-xs font-semibold text-[var(--scry-accent-text)]"
        >
          <Tag className="size-3 shrink-0" aria-hidden="true" />
          <span className="truncate">{label}</span>
          <button
            id={`${idPrefix}-tag-remove-${label.replace(/\s+/g, "-")}`}
            type="button"
            aria-label={t("title.tagsRemove", { label })}
            title={t("title.tagsRemove", { label })}
            onClick={() => removeLabel(label)}
            disabled={disabled}
            className="rounded-[5px] p-0.5 transition hover:bg-[rgba(var(--scry-accent-rgb),0.28)] disabled:opacity-50"
          >
            <X className="h-3 w-3" aria-hidden="true" />
          </button>
        </span>
      ))}
    </div>
  ) : (
    <p className="text-sm text-muted-foreground">
      {emptyValueText ?? t("title.tagsNone")}
    </p>
  );

  const selector = registryIsEmpty ? (
    // No free text means an empty registry has nothing to offer, so the
    // picker says where tags come from instead of showing a dead control.
    <p className="text-xs text-muted-foreground">{t("title.tagsEmptyRegistry")}</p>
  ) : (
    <Select
      value={ADD_PLACEHOLDER_VALUE}
      onValueChange={addLabel}
      disabled={disabled || loading || options.length === 0}
    >
      <SelectTrigger id={`${idPrefix}-tags-add`} className="h-9 w-full">
        <SelectValue placeholder={t("title.tagsAdd")} />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ADD_PLACEHOLDER_VALUE}>
          {options.length === 0 ? t("title.tagsAllApplied") : t("title.tagsAdd")}
        </SelectItem>
        {options.map((label) => (
          <SelectItem key={label} value={label}>
            {label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );

  if (layout === "table") {
    return (
      <>
        <td className="w-[38%] px-4 py-3 align-middle text-sm text-foreground sm:px-5">
          <div id={`${idPrefix}-tags`} className="flex min-w-0 items-center">
            {appliedTags}
          </div>
        </td>
        <td className="w-[38%] px-4 py-3 align-middle sm:px-5">
          <div className="min-w-40">{selector}</div>
        </td>
      </>
    );
  }

  return (
    <div
      className={
        layout === "horizontal"
          ? "grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] items-center"
          : "space-y-2"
      }
      id={`${idPrefix}-tags`}
    >
      <div
        className={
          layout === "horizontal" ? "flex min-w-0 items-center pr-5" : "min-w-0"
        }
      >
        {appliedTags}
      </div>
      <div className="min-w-0">{selector}</div>
    </div>
  );
}

export type TitleTagsEditorProps = {
  titleId: string;
  tags: readonly string[] | null | undefined;
  idPrefix: string;
  /**
   * Refreshes the title detail once the patch lands, the same way a title
   * options save does — the picker renders from the title's own bag rather
   * than from local state, so the refresh is what makes the change stick.
   */
  onTitleChanged?: () => Promise<void> | void;
  disabled?: boolean;
  layout?: "stacked" | "horizontal" | "table";
  showLabel?: boolean;
};

/**
 * The per-title picker. Sends only the difference, so a concurrent options save
 * that rewrites the reserved `scryer:` entries cannot be clobbered by a tag
 * save built from a stale bag.
 */
export function TitleTagsEditor({
  titleId,
  tags,
  idPrefix,
  onTitleChanged,
  disabled = false,
  layout = "stacked",
  showLabel = true,
}: TitleTagsEditorProps) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const { definitions, loading } = useTitleTagDefinitions();
  const [saving, setSaving] = React.useState(false);

  const applyTags = React.useCallback(
    async (labels: string[]) => {
      const delta = titleTagsDelta(tags, labels);
      if (isEmptyTitleTagsDelta(delta)) {
        return;
      }
      setSaving(true);
      try {
        const { error } = await client
          .mutation(updateTitleTagsMutation, {
            input: { titleIds: [titleId], add: delta.add, remove: delta.remove },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        await onTitleChanged?.();
      } catch (error: unknown) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setSaving(false);
      }
    },
    [client, onTitleChanged, setGlobalStatus, t, tags, titleId],
  );

  const picker = (
    <TitleTagsPicker
      value={tags}
      onChange={(labels) => void applyTags(labels)}
      definitions={definitions}
      loading={loading}
      disabled={disabled || saving}
      idPrefix={idPrefix}
      layout={layout}
    />
  );

  if (layout === "table") {
    return picker;
  }

  return (
    <div className="min-w-0">
      {showLabel ? (
        <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Tag aria-hidden="true" className="size-3.5" />
          {t("title.tagsLabel")}
        </label>
      ) : null}
      {picker}
    </div>
  );
}

export type SeriesMovieTagsEditorProps = {
  seriesMovieLinkId: string;
  tags: readonly string[] | null | undefined;
  idPrefix: string;
  /**
   * Refreshes the series detail once the patch lands, the same way the
   * monitoring toggle beside it does.
   */
  onLinkChanged?: () => Promise<void> | void;
  disabled?: boolean;
};

/**
 * The per-series-movie picker.
 *
 * Identical in behaviour to {@link TitleTagsEditor} and deliberately sharing
 * its picker and its delta helper; only the mutation differs, because a series
 * movie is a `series_movie_links` row rather than a title and carries its own
 * bag.
 */
export function SeriesMovieTagsEditor({
  seriesMovieLinkId,
  tags,
  idPrefix,
  onLinkChanged,
  disabled = false,
}: SeriesMovieTagsEditorProps) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const { definitions, loading } = useTitleTagDefinitions();
  const [saving, setSaving] = React.useState(false);

  const applyTags = React.useCallback(
    async (labels: string[]) => {
      const delta = titleTagsDelta(tags, labels);
      if (isEmptyTitleTagsDelta(delta)) {
        return;
      }
      setSaving(true);
      try {
        const { error } = await client
          .mutation(updateSeriesMovieTagsMutation, {
            input: {
              seriesMovieLinkIds: [seriesMovieLinkId],
              add: delta.add,
              remove: delta.remove,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        await onLinkChanged?.();
      } catch (error: unknown) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setSaving(false);
      }
    },
    [client, onLinkChanged, seriesMovieLinkId, setGlobalStatus, t, tags],
  );

  return (
    <div className="min-w-0">
      <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Tag aria-hidden="true" className="size-3.5" />
        {t("title.tagsLabel")}
      </label>
      <TitleTagsPicker
        value={tags}
        onChange={(labels) => void applyTags(labels)}
        definitions={definitions}
        loading={loading}
        disabled={disabled || saving}
        idPrefix={idPrefix}
      />
    </div>
  );
}
