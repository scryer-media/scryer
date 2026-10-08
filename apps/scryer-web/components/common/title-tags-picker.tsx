import * as React from "react";
import { useNavigate } from "react-router";
import { useClient } from "urql";
import { ChevronDown, Tag, X } from "lucide-react";

import { Command, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { selectContentClassName, selectTriggerClassName } from "@/components/ui/select";
import { titleTagDefinitionsQuery } from "@/lib/graphql/queries";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  createTitleTagDefinitionMutation,
  updateSeriesMovieTagsMutation,
  updateTitleTagsMutation,
} from "@/lib/graphql/mutations";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { useTitleTagDefinitions, refreshTitleTagConsumers } from "@/lib/hooks/use-title-tag-definitions";
import type { TitleTagDefinition } from "@/lib/types/title-tags";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import {
  normalizeTitleTagLabel,
  titleTagLabelErrorKey,
  availableTitleTagLabels,
  isEmptyTitleTagsDelta,
  titleTagsDelta,
  userTitleTags,
} from "@/lib/utils/title-tags";

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

/** Shared searchable registry picker with permission-gated inline creation. */
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
  const navigate = useNavigate();
  const client = useClient();
  const [open, setOpen] = React.useState(false);
  const [search, setSearch] = React.useState("");
  const [creating, setCreating] = React.useState(false);
  const [createError, setCreateError] = React.useState<string | null>(null);
  const busyRef = React.useRef(false);
  const current = React.useRef({ value, excludedLabels, onChange, disabled });
  React.useEffect(() => { current.current = { value, excludedLabels, onChange, disabled }; }, [value, excludedLabels, onChange, disabled]);
  const sessionUser = useSessionUser();
  const canManageRegistry = hasAppPermission(
    sessionUser,
    APP_PERMISSIONS.manageCatalogSettings,
  );
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

  const addLabel = (label: string) => {
    const latest = current.current;
    if (latest.disabled || userTitleTags(latest.excludedLabels).includes(label)) return;
    latest.onChange(userTitleTags([...userTitleTags(latest.value), label]));
    setOpen(false);
    setSearch("");
  };
  const candidate = normalizeTitleTagLabel(search);
  const candidateError = titleTagLabelErrorKey(candidate);
  const canCreate = canManageRegistry && candidate.length > 0 && !candidateError
    && !definitions.some((definition) => normalizeTitleTagLabel(definition.label) === candidate)
    && !excluded.has(candidate) && !applied.includes(candidate);
  const createAndSelect = async () => {
    if (!canCreate || disabled || loading || busyRef.current) return;
    busyRef.current = true;
    setCreating(true);
    setCreateError(null);
    try {
      const result = await client.mutation(createTitleTagDefinitionMutation, { input: { label: candidate, description: null } }).toPromise();
      let label: string | undefined = result.data?.createTitleTagDefinition?.definition?.label;
      let refreshedDefinitions: TitleTagDefinition[] | undefined;
      if (result.error) {
        if (!result.error.message.toLowerCase().includes("already exists")) throw result.error;
        const refreshed = await client.query(titleTagDefinitionsQuery, {}, { requestPolicy: "network-only" }).toPromise();
        if (refreshed.error) throw refreshed.error;
        refreshedDefinitions = refreshed.data?.titleTagDefinitions;
        label = refreshed.data?.titleTagDefinitions?.find((definition: TitleTagDefinition) => normalizeTitleTagLabel(definition.label) === candidate)?.label;
        if (!label) throw result.error;
      }
      if (!label) throw new Error(t("settings.titleTagSaveError"));
      refreshTitleTagConsumers(client, refreshedDefinitions);
      addLabel(label);
    } catch (error) {
      setCreateError(error instanceof Error ? error.message : t("settings.titleTagSaveError"));
    } finally {
      busyRef.current = false;
      setCreating(false);
    }
  };

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

  const selector = registryIsEmpty && !canManageRegistry ? (
    <p className="text-xs text-muted-foreground">{t("title.tagsEmptyRegistry")}</p>
  ) : (
    <Popover open={open && !disabled} onOpenChange={(next) => { if (!creating) { setOpen(next); setCreateError(null); } }} modal>
      <PopoverTrigger asChild>
        <button type="button" id={`${idPrefix}-tags-add`} disabled={disabled || loading || creating}
          className={selectTriggerClassName({ className: layout === "table" ? "ml-auto h-9 w-[70%]" : "h-9 w-full" })}>
          <span className="min-w-0 truncate text-left">{t("title.tagsAdd")}</span>
          <ChevronDown className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
        </button>
      </PopoverTrigger>
      <PopoverContent className={selectContentClassName("z-[90] w-[var(--radix-popover-trigger-width)] min-w-64 p-0")}>
        <Command shouldFilter={false}>
          <CommandInput value={search} onValueChange={(value) => { setSearch(value); setCreateError(null); }} placeholder={t("title.tagsSearch")} disabled={creating} />
          <CommandList>
            {options.filter((label) => label.includes(normalizeTitleTagLabel(search))).map((label) => (
              <CommandItem key={label} value={label} onSelect={() => addLabel(label)} disabled={creating}>{label}</CommandItem>
            ))}
            {canCreate ? <CommandItem value={`create-${candidate}`} onSelect={() => void createAndSelect()} disabled={creating}>
              {t("title.tagsCreateSelect", { label: candidate })}
            </CommandItem> : null}
            {canManageRegistry ? <CommandItem id={`${idPrefix}-tags-manage`} value="manage-registry" disabled={creating} onSelect={() => { setOpen(false); void navigate("/settings/tags"); }}>{t("title.tagsCreateMore")}</CommandItem> : null}
          </CommandList>
        </Command>
        {createError || (candidate && candidateError) ? <p role="alert" className="p-3 text-sm text-destructive">{createError ?? t(candidateError!)}</p> : null}
      </PopoverContent>
    </Popover>
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
