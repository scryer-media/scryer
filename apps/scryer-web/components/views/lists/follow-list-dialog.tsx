import * as React from "react";
import { Eye, ListPlus } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { CheckboxField } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { SingleSelectField } from "@/components/ui/select";
import { ActionTooltip } from "@/components/ui/tooltip";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet } from "@/lib/facets/helpers";
import { useTitleTagDefinitions } from "@/lib/hooks/use-title-tag-definitions";
import type {
  ListMode,
  ListPreview,
  ListProviderItem,
  ListProviderManifest,
  ListSourceDraft,
  ListSubscription,
  ListSubscriptionDraft,
} from "@/lib/types/lists";
import type { Facet } from "@/lib/types/titles";
import {
  defaultListRoute,
  emptyListDraft,
  isListModeSelectable,
  LIST_KINDS,
  LIST_ON_LEAVE_OPTIONS,
  listDraftProblems,
  listIntervalParts,
  listKindLabelKey,
  listModeHelpKey,
  listModeLabelKey,
  listOnLeaveLabelKey,
  missingSourceParams,
  PUBLIC_LIST_MODES,
  setListParam,
  subscriptionToDraft,
} from "@/lib/utils/lists";

import { ListFiltersFields } from "./list-filters-fields";
import { ListPreviewSummary } from "./list-preview-summary";
import { ListRouteCard } from "./list-route-card";
import { ProviderTile } from "./provider-tile";
import type { ListRouteOptions } from "./lists-view";

export type FollowListTarget =
  | {
      kind: "new";
      source: ListSourceDraft;
      manifest: ListProviderManifest | null;
      item: ListProviderItem | null;
      name: string;
      kinds: Facet[];
      preview: ListPreview | null;
    }
  | { kind: "edit"; subscription: ListSubscription; manifest: ListProviderManifest | null };

type FollowListDialogProps = {
  target: FollowListTarget | null;
  routeOptions: ListRouteOptions;
  onClose: () => void;
  onPreviewSource: (source: ListSourceDraft) => Promise<ListPreview | null>;
  onSubscribe: (source: ListSourceDraft, draft: ListSubscriptionDraft) => Promise<boolean>;
  onUpdate: (id: string, draft: ListSubscriptionDraft) => Promise<boolean>;
};

const SECTION_HEADING = "text-[13px] font-semibold uppercase tracking-[0.06em] text-[var(--scry-muted)]";
const ID = "follow-list";

function initialDraft(target: FollowListTarget, routeOptions: ListRouteOptions): ListSubscriptionDraft {
  if (target.kind === "edit") return subscriptionToDraft(target.subscription);
  const draft = emptyListDraft(target.name, target.kinds);
  draft.routes = target.kinds.map((kind) =>
    defaultListRoute(kind, routeOptions.libraries, defaultMonitorTypeForFacet(kind)),
  );
  return draft;
}

export function FollowListDialog(props: FollowListDialogProps) {
  const { target, onClose } = props;
  return (
    <Dialog open={target !== null} onOpenChange={(open) => (!open ? onClose() : undefined)}>
      {target ? <FollowListDialogBody key={dialogKey(target)} {...props} target={target} /> : null}
    </Dialog>
  );
}

function dialogKey(target: FollowListTarget): string {
  return target.kind === "edit"
    ? `edit:${target.subscription.id}`
    : `new:${target.source.provider}:${target.source.sourceType}:${target.source.url ?? ""}`;
}

function FollowListDialogBody({
  target,
  routeOptions,
  onClose,
  onPreviewSource,
  onSubscribe,
  onUpdate,
}: FollowListDialogProps & { target: FollowListTarget }) {
  const t = useTranslate();
  const { definitions: tagDefinitions, loading: tagsLoading } = useTitleTagDefinitions();
  const [draft, setDraft] = React.useState(() => initialDraft(target, routeOptions));
  const [source, setSource] = React.useState<ListSourceDraft | null>(
    target.kind === "new" ? target.source : null,
  );
  const [preview, setPreview] = React.useState<ListPreview | null>(
    target.kind === "new" ? target.preview : null,
  );
  const [previewing, setPreviewing] = React.useState(false);
  const [saving, setSaving] = React.useState(false);

  const manifest = target.manifest;
  const item = target.kind === "new" ? target.item : null;
  const paramDefinitions = item?.params ?? [];
  const offeredKinds: readonly Facet[] =
    target.kind === "new" && target.kinds.length > 0
      ? target.kinds
      : manifest?.coverage.length
        ? manifest.coverage
        : LIST_KINDS;
  const intervalSeconds =
    target.kind === "edit" ? target.subscription.intervalSeconds : (item?.defaultIntervalSeconds ?? null);
  const interval = intervalSeconds ? listIntervalParts(intervalSeconds) : null;

  const missingParams = source ? missingSourceParams(paramDefinitions, source.params) : [];
  const problems = listDraftProblems(draft);
  const canSave = !saving && problems.length === 0 && missingParams.length === 0;

  const toggleKind = (kind: Facet, checked: boolean) => {
    setDraft((current) => {
      const kinds = checked
        ? LIST_KINDS.filter((entry) => entry === kind || current.kinds.includes(entry))
        : current.kinds.filter((entry) => entry !== kind);
      const routes = current.routes.some((route) => route.kind === kind)
        ? current.routes
        : [...current.routes, defaultListRoute(kind, routeOptions.libraries, defaultMonitorTypeForFacet(kind))];
      return { ...current, kinds, routes };
    });
  };

  const runPreview = async () => {
    if (!source) return;
    setPreviewing(true);
    try {
      setPreview(await onPreviewSource(source));
    } finally {
      setPreviewing(false);
    }
  };

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      const ok =
        target.kind === "edit"
          ? await onUpdate(target.subscription.id, draft)
          : source
            ? await onSubscribe(source, draft)
            : false;
      if (ok) onClose();
    } finally {
      setSaving(false);
    }
  };

  const sourceLabel =
    target.kind === "edit"
      ? target.subscription.source.sourceType
      : (item?.name ?? target.source.sourceType);

  return (
    <DialogContent id={`${ID}-dialog`} className="max-h-[90vh] gap-0 overflow-y-auto p-0 sm:max-w-3xl">
      <DialogHeader className="border-b border-[var(--scry-border3)] p-5 sm:p-6">
        <div className="flex items-center gap-3">
          <ProviderTile provider={manifest} />
          <div className="min-w-0">
            <DialogTitle className="font-display text-[19px]">
              {target.kind === "edit" ? t("lists.follow.editTitle") : t("lists.follow.title")}
            </DialogTitle>
            <DialogDescription className="truncate">
              {[manifest?.name, sourceLabel].filter(Boolean).join(" · ")}
            </DialogDescription>
          </div>
        </div>
      </DialogHeader>

      <div className="space-y-6 p-5 sm:p-6">
        {source && paramDefinitions.length > 0 ? (
          <section className="space-y-3">
            <h3 className={SECTION_HEADING}>{t("lists.follow.sourceHeading")}</h3>
            <div className="grid gap-3 sm:grid-cols-2">
              {paramDefinitions.map((definition) => {
                const value = source.params.find((param) => param.key === definition.key)?.value ?? "";
                const onValue = (next: string) => {
                  setSource((current) =>
                    current ? { ...current, params: setListParam(current.params, definition.key, next) } : current,
                  );
                  setPreview(null);
                };
                const fieldId = `${ID}-param-${definition.key}`;
                return definition.options.length > 0 ? (
                  <SingleSelectField
                    key={definition.key}
                    id={fieldId}
                    label={definition.label}
                    required={definition.required}
                    value={value}
                    options={definition.options.map((option) => ({ value: option, label: option }))}
                    onValueChange={onValue}
                  />
                ) : (
                  <label key={definition.key} className="space-y-1.5" htmlFor={fieldId}>
                    <span className="block text-sm font-medium text-[var(--scry-ink2)]">
                      {definition.label}
                      {definition.required ? <span aria-hidden="true"> *</span> : null}
                    </span>
                    <Input
                      id={fieldId}
                      value={value}
                      inputMode={definition.type === "URL" ? "url" : undefined}
                      onChange={(event) => onValue(event.target.value)}
                    />
                  </label>
                );
              })}
            </div>
          </section>
        ) : null}

        <section className="space-y-3">
          <label className="block space-y-1.5" htmlFor={`${ID}-name`}>
            <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.follow.name")}</span>
            <Input
              id={`${ID}-name`}
              value={draft.name}
              onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))}
            />
          </label>
          <div className="space-y-2">
            <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.follow.kinds")}</span>
            <div className="flex flex-wrap gap-x-6 gap-y-2">
              {offeredKinds.map((kind) => (
                <CheckboxField
                  key={kind}
                  id={`${ID}-kind-${kind.toLowerCase()}`}
                  label={t(listKindLabelKey(kind))}
                  checked={draft.kinds.includes(kind)}
                  onCheckedChange={(checked) => toggleKind(kind, checked === true)}
                />
              ))}
            </div>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className={SECTION_HEADING}>{t("lists.follow.modeHeading")}</h3>
          <RadioGroup
            value={draft.mode}
            onValueChange={(mode) => setDraft((current) => ({ ...current, mode: mode as ListMode }))}
            className="grid gap-2 sm:grid-cols-2"
          >
            {PUBLIC_LIST_MODES.map((mode) => {
              const selectable = isListModeSelectable(mode);
              const option = (
                <label
                  key={mode}
                  htmlFor={`${ID}-mode-${mode.toLowerCase()}`}
                  className="flex w-full items-start gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3 text-sm"
                >
                  <RadioGroupItem
                    id={`${ID}-mode-${mode.toLowerCase()}`}
                    value={mode}
                    disabled={!selectable}
                    className="mt-0.5"
                  />
                  <span className={selectable ? undefined : "opacity-60"}>
                    <span className="block font-medium text-[var(--scry-ink2)]">{t(listModeLabelKey(mode))}</span>
                    <span className="block text-xs text-[var(--scry-muted)]">{t(listModeHelpKey(mode))}</span>
                  </span>
                </label>
              );
              return selectable ? (
                option
              ) : (
                <ActionTooltip key={mode} content={t("lists.mode.discoverUnavailable")} wrapperClassName="flex">
                  {option}
                </ActionTooltip>
              );
            })}
          </RadioGroup>
        </section>

        {draft.kinds.length > 0 ? (
          <section className="space-y-3">
            <h3 className={SECTION_HEADING}>{t("lists.follow.routesHeading")}</h3>
            <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.follow.routesHelp")}</p>
            {draft.kinds.map((kind) => {
              const route =
                draft.routes.find((entry) => entry.kind === kind) ??
                defaultListRoute(kind, routeOptions.libraries, defaultMonitorTypeForFacet(kind));
              return (
                <ListRouteCard
                  key={kind}
                  kind={kind}
                  route={route}
                  libraries={routeOptions.libraries}
                  qualityProfiles={routeOptions.qualityProfiles}
                  tagDefinitions={tagDefinitions}
                  tagsLoading={tagsLoading}
                  idPrefix={ID}
                  onChange={(next) =>
                    setDraft((current) => ({
                      ...current,
                      routes: [...current.routes.filter((entry) => entry.kind !== kind), next],
                    }))
                  }
                />
              );
            })}
          </section>
        ) : null}

        <section className="space-y-3">
          <h3 className={SECTION_HEADING}>{t("lists.follow.filtersHeading")}</h3>
          <ListFiltersFields
            filters={draft.filters}
            idPrefix={ID}
            onChange={(filters) => setDraft((current) => ({ ...current, filters }))}
          />
        </section>

        <section className="space-y-3">
          <h3 className={SECTION_HEADING}>{t("lists.follow.syncHeading")}</h3>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="space-y-1.5" htmlFor={`${ID}-max-per-sync`}>
              <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.follow.maxPerSync")}</span>
              <Input
                id={`${ID}-max-per-sync`}
                {...integerInputProps}
                placeholder={t("lists.follow.maxPerSyncPlaceholder")}
                value={draft.maxPerSync === null ? "" : String(draft.maxPerSync)}
                onChange={(event) => {
                  const digits = sanitizeDigits(event.target.value);
                  setDraft((current) => ({ ...current, maxPerSync: digits ? Number(digits) : null }));
                }}
              />
              <span className="block text-xs text-[var(--scry-muted)]">{t("lists.follow.maxPerSyncHelp")}</span>
            </label>
            <SingleSelectField
              id={`${ID}-on-leave`}
              label={t("lists.follow.onLeave")}
              description={t("lists.follow.onLeaveHelp")}
              value={draft.onLeave}
              options={LIST_ON_LEAVE_OPTIONS.map((value) => ({ value, label: t(listOnLeaveLabelKey(value)) }))}
              onValueChange={(value) =>
                setDraft((current) => ({ ...current, onLeave: value as ListSubscriptionDraft["onLeave"] }))
              }
            />
          </div>
          {interval ? (
            <p id={`${ID}-interval`} className="text-[12.5px] text-[var(--scry-muted)]">
              {t("lists.follow.interval", { interval: t(interval.key, { count: interval.count }) })}
            </p>
          ) : null}
        </section>

        {source ? (
          <section className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className={SECTION_HEADING}>{t("lists.preview.heading")}</h3>
              <Button
                id={`${ID}-preview-button`}
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void runPreview()}
                disabled={previewing || missingParams.length > 0}
              >
                {previewing ? <LoadingMark className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                {t("lists.preview.run")}
              </Button>
            </div>
            {preview ? (
              <ListPreviewSummary preview={preview} idPrefix={ID} />
            ) : (
              <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.preview.notRun")}</p>
            )}
          </section>
        ) : null}

        {problems.length > 0 || missingParams.length > 0 ? (
          <ul id={`${ID}-problems`} className="space-y-1 text-[12.5px] text-[var(--scry-warning-text)]">
            {missingParams.length > 0 ? <li>{t("lists.follow.problem.params")}</li> : null}
            {problems.map((key) => (
              <li key={key}>{t(key)}</li>
            ))}
          </ul>
        ) : null}
      </div>

      <DialogFooter className="border-t border-[var(--scry-border3)] p-5 sm:p-6">
        <Button id={`${ID}-cancel`} type="button" variant="outline" onClick={onClose} disabled={saving}>
          {t("label.cancel")}
        </Button>
        <Button id={`${ID}-submit`} type="button" onClick={() => void save()} disabled={!canSave}>
          {saving ? <LoadingMark className="h-4 w-4" /> : <ListPlus className="h-4 w-4" />}
          {target.kind === "edit" ? t("label.save") : t("lists.follow.submit")}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}
