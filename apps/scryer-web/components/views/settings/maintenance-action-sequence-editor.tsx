import * as React from "react";
import { ChevronDown, ChevronUp, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import { TitleTagsPicker } from "@/components/common/title-tags-picker";
import { useTitleTagDefinitions } from "@/lib/hooks/use-title-tag-definitions";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionSequence,
  MaintenanceActionStep,
  MaintenanceActionStepDescriptor,
  MaintenanceActionStepKind,
  MaintenanceRiskClass,
  MaintenanceRuleScope,
} from "@/lib/types/maintenance-rule-sets";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  actionStepKindLabelKey,
  effectArmingLabelKey,
  maintenanceActionSequenceStepId,
  riskClassBadgeTone,
  riskClassLabelKey,
} from "@/lib/utils/maintenance-rule-sets";
import { Badge } from "@/components/ui/badge";

type MaintenanceQualityProfileOption = { id: string; name: string };

type MaintenanceActionSequenceEditorProps = {
  sequence: MaintenanceActionSequence;
  descriptors: MaintenanceActionStepDescriptor[];
  subjectKind: MaintenanceRuleScope;
  storageRootId: string;
  qualityProfiles: MaintenanceQualityProfileOption[];
  onChange: (sequence: MaintenanceActionSequence) => void;
};

type MaintenanceActionSequencePreset =
  | "PROFILE_THEN_CONDITIONAL_SEARCH"
  | "UNMONITOR_DESCENDANTS_THEN_DELETE_FILES";

/// A response may be missing while a saved sequence already contains steps.
/// Never make those effects disappear from the editor: callers use this to
/// hold preview and save until the authoritative catalog is available again.
export function sequenceHasUnresolvedSteps(
  sequence: MaintenanceActionSequence,
  descriptors: MaintenanceActionStepDescriptor[],
): boolean {
  return sequence.steps.some(
    (step) => !descriptors.some((descriptor) => descriptor.kind === step.kind),
  );
}

function stepKindLabelKey(kind: MaintenanceActionStepKind): string {
  return actionStepKindLabelKey(kind) ?? kind;
}

function emptyStepParameters() {
  return {
    includeDescendants: null,
    targetQualityProfileId: null,
    searchCondition: null,
    tags: [],
  };
}

function supportsRuleScope(
  descriptor: MaintenanceActionStepDescriptor,
  scope: MaintenanceRuleScope,
): boolean {
  switch (scope) {
    case "TITLE":
      return (
        descriptor.supportedSubjects.includes("MOVIE") ||
        descriptor.supportedSubjects.includes("SHOW")
      );
    case "SEASON":
      return descriptor.supportedSubjects.includes("SEASON");
    case "EPISODE":
      return descriptor.supportedSubjects.includes("EPISODE");
  }
}

const RISK_ORDER: Record<MaintenanceRiskClass, number> = {
  NONE: 0,
  LOW: 1,
  MEDIUM: 2,
  HIGH: 3,
};

function sequenceTargetLabelKey(
  sequence: MaintenanceActionSequence,
  subjectKind: MaintenanceRuleScope,
): string {
  const unmonitor = sequence.steps.find((step) => step.kind === "UNMONITOR");
  if (unmonitor?.parameters.includeDescendants) {
    switch (subjectKind) {
      case "TITLE":
        return "settings.maintenanceSequenceTargetTitleDescendants";
      case "SEASON":
        return "settings.maintenanceSequenceTargetSeasonDescendants";
      case "EPISODE":
        return "settings.maintenanceSequenceTargetEpisode";
    }
  }
  switch (subjectKind) {
    case "TITLE":
      return "settings.maintenanceSequenceTargetTitle";
    case "SEASON":
      return "settings.maintenanceSequenceTargetSeason";
    case "EPISODE":
      return "settings.maintenanceSequenceTargetEpisode";
  }
}

function newStep(
  descriptor: MaintenanceActionStepDescriptor,
  subjectKind: MaintenanceRuleScope,
): MaintenanceActionStep {
  return {
    id: maintenanceActionSequenceStepId(),
    kind: descriptor.kind,
    parameters: {
      ...emptyStepParameters(),
      includeDescendants: descriptor.parameterSchema === "unmonitor"
        ? subjectKind === "TITLE"
          ? false
          : true
        : null,
      searchCondition:
        descriptor.parameterSchema === "search" ? "UNCONDITIONAL" : null,
    },
  };
}

function presetLabelKey(preset: MaintenanceActionSequencePreset): string {
  switch (preset) {
    case "PROFILE_THEN_CONDITIONAL_SEARCH":
      return "settings.maintenanceSequencePresetProfileThenSearch";
    case "UNMONITOR_DESCENDANTS_THEN_DELETE_FILES":
      return "settings.maintenanceSequencePresetUnmonitorThenDelete";
  }
}

function presetStepKinds(
  preset: MaintenanceActionSequencePreset,
): MaintenanceActionStepKind[] {
  switch (preset) {
    case "PROFILE_THEN_CONDITIONAL_SEARCH":
      return ["CHANGE_QUALITY_PROFILE", "SEARCH"];
    case "UNMONITOR_DESCENDANTS_THEN_DELETE_FILES":
      return ["UNMONITOR", "DELETE_FILES"];
  }
}

function applyPreset(
  preset: MaintenanceActionSequencePreset,
  sequence: MaintenanceActionSequence,
  descriptors: MaintenanceActionStepDescriptor[],
  subjectKind: MaintenanceRuleScope,
  storageRootId: string,
): MaintenanceActionSequence | null {
  const steps = presetStepKinds(preset).map((kind) => {
    const descriptor = descriptors.find(
      (item) =>
        item.kind === kind &&
        supportsRuleScope(item, subjectKind) &&
        (!storageRootId || item.storageRootAllowed),
    );
    if (!descriptor) return null;
    const step = newStep(descriptor, subjectKind);
    if (step.kind === "UNMONITOR") {
      step.parameters.includeDescendants = true;
    }
    if (step.kind === "SEARCH") {
      step.parameters.searchCondition = "PREVIOUS_PROFILE_CHANGED";
    }
    return step;
  });
  return steps.every((step): step is MaintenanceActionStep => step !== null)
    ? { ...sequence, steps }
    : null;
}

function ActionSequenceStep({
  step,
  descriptor,
  index,
  count,
  subjectKind,
  qualityProfiles,
  update,
  remove,
  move,
}: {
  step: MaintenanceActionStep;
  descriptor: MaintenanceActionStepDescriptor;
  index: number;
  count: number;
  subjectKind: MaintenanceRuleScope;
  qualityProfiles: MaintenanceQualityProfileOption[];
  update: (next: MaintenanceActionStep) => void;
  remove: () => void;
  move: (offset: -1 | 1) => void;
}) {
  const t = useTranslate();
  const needsTags = descriptor.parameterSchema === "tags";
  const needsProfile = descriptor.parameterSchema === "change_quality_profile";
  const needsSearchCondition = descriptor.parameterSchema === "search";
  const mayChooseDescendants =
    subjectKind === "TITLE" && descriptor.parameterSchema === "unmonitor";
  const { definitions, loading } = useTitleTagDefinitions({ enabled: needsTags });

  const replaceParameters = (parameters: MaintenanceActionStep["parameters"]) =>
    update({ ...step, parameters });

  return (
    <li
      id={selectorId("settings-maintenance-sequence-step", step.id)}
      className="rounded border border-border bg-card p-3"
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{t(stepKindLabelKey(step.kind))}</span>
            <Badge tone={riskClassBadgeTone(descriptor.riskClass)}>
              {descriptor.riskClass}
            </Badge>
            {descriptor.terminal ? (
              <Badge tone="negative">{t("settings.maintenanceSequenceTerminal")}</Badge>
            ) : null}
            <Badge tone="neutral">
              {descriptor.completionPolicy === "accepted"
                ? t("settings.maintenanceSequenceCompletionAccepted")
                : t("settings.maintenanceSequenceCompletionCompleted")}
            </Badge>
          </div>
          {descriptor.requires.length > 0 ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {t("settings.maintenanceSequenceDependencies", {
                steps: descriptor.requires.join(", "),
              })}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <IconButton
            type="button"
            label={t("label.moveUp")}
            disabled={index === 0}
            onClick={() => move(-1)}
          >
            <ChevronUp className="h-4 w-4" />
          </IconButton>
          <IconButton
            type="button"
            label={t("label.moveDown")}
            disabled={index === count - 1}
            onClick={() => move(1)}
          >
            <ChevronDown className="h-4 w-4" />
          </IconButton>
          <IconButton
            type="button"
            label={t("label.delete")}
            onClick={remove}
          >
            <Trash2 className="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      {mayChooseDescendants ? (
        <label className="mt-3 flex items-center gap-2 text-sm">
          <Checkbox
            checked={Boolean(step.parameters.includeDescendants)}
            onCheckedChange={(checked) =>
              replaceParameters({
                ...step.parameters,
                includeDescendants: checked === true,
              })
            }
          />
          {t("settings.maintenanceSequenceIncludeDescendants")}
        </label>
      ) : null}

      {needsProfile ? (
        <div className="mt-3">
          {qualityProfiles.length > 0 ? (
            <SingleSelectField
              id={selectorId("settings-maintenance-sequence-profile", step.id)}
              label={t("settings.maintenanceRuleTargetProfile")}
              placeholder={t("settings.maintenanceRuleTargetProfileHelp")}
              value={step.parameters.targetQualityProfileId ?? ""}
              onValueChange={(targetQualityProfileId) =>
                replaceParameters({ ...step.parameters, targetQualityProfileId })
              }
              options={qualityProfiles.map((profile) => ({
                value: profile.id,
                label: profile.name,
              }))}
            />
          ) : (
            <label>
              <Label className="mb-2 block">
                {t("settings.maintenanceRuleTargetProfile")}
              </Label>
              <Input
                id={selectorId("settings-maintenance-sequence-profile", step.id)}
                value={step.parameters.targetQualityProfileId ?? ""}
                onChange={(event) =>
                  replaceParameters({
                    ...step.parameters,
                    targetQualityProfileId: event.target.value,
                  })
                }
              />
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.maintenanceRuleTargetProfileIdHelp")}
              </p>
            </label>
          )}
        </div>
      ) : null}

      {needsTags ? (
        <div className="mt-3">
          <Label className="mb-2 block">{t("settings.maintenanceRuleTags")}</Label>
          <TitleTagsPicker
            value={step.parameters.tags}
            onChange={(tags) => replaceParameters({ ...step.parameters, tags })}
            definitions={definitions}
            loading={loading}
            idPrefix={selectorId("settings-maintenance-sequence-tags", step.id)}
            emptyValueText={t("settings.maintenanceRuleTagsNone")}
          />
        </div>
      ) : null}

      {needsSearchCondition ? (
        <div className="mt-3">
          <SingleSelectField
            id={selectorId("settings-maintenance-sequence-search-condition", step.id)}
            label={t("settings.maintenanceSequenceSearchCondition")}
            value={step.parameters.searchCondition ?? "UNCONDITIONAL"}
            onValueChange={(searchCondition) =>
              replaceParameters({
                ...step.parameters,
                searchCondition: searchCondition as MaintenanceActionStep["parameters"]["searchCondition"],
              })
            }
            options={[
              {
                value: "UNCONDITIONAL",
                label: t("settings.maintenanceSequenceSearchUnconditional"),
              },
              {
                value: "PREVIOUS_PROFILE_CHANGED",
                label: t("settings.maintenanceSequenceSearchPreviousProfileChanged"),
              },
            ]}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.maintenanceSequenceSearchRequested")}
          </p>
        </div>
      ) : null}
    </li>
  );
}

/// Ordered schema-2 action editor. All policy choices come from the supplied
/// server descriptors. The component only edits stable IDs and parameter
/// values; the API validates ordering, completion coverage, conflicts, and
/// storage-root compatibility on every preview and save.
export function MaintenanceActionSequenceEditor({
  sequence,
  descriptors,
  subjectKind,
  storageRootId,
  qualityProfiles,
  onChange,
}: MaintenanceActionSequenceEditorProps) {
  const t = useTranslate();
  const available = React.useMemo(
    () =>
      descriptors.filter(
        (descriptor) =>
          supportsRuleScope(descriptor, subjectKind) &&
          (!storageRootId || descriptor.storageRootAllowed) &&
          !sequence.steps.some((step) => step.kind === descriptor.kind),
      ),
    [descriptors, sequence.steps, storageRootId, subjectKind],
  );
  const [selectedKind, setSelectedKind] = React.useState<MaintenanceActionStepKind | "">("");
  const [selectedPreset, setSelectedPreset] =
    React.useState<MaintenanceActionSequencePreset | "">("");
  const presetOptions = React.useMemo(
    () =>
      (
        [
          "PROFILE_THEN_CONDITIONAL_SEARCH",
          "UNMONITOR_DESCENDANTS_THEN_DELETE_FILES",
        ] as const
      ).filter((preset) =>
        presetStepKinds(preset).every((kind) =>
          descriptors.some(
            (descriptor) =>
              descriptor.kind === kind &&
              supportsRuleScope(descriptor, subjectKind) &&
              (!storageRootId || descriptor.storageRootAllowed),
          ),
        ),
      ),
    [descriptors, storageRootId, subjectKind],
  );
  const summary = React.useMemo(() => {
    const selected = sequence.steps
      .map((step) => descriptors.find((descriptor) => descriptor.kind === step.kind))
      .filter((descriptor): descriptor is MaintenanceActionStepDescriptor => Boolean(descriptor));
    const risk = selected.reduce<MaintenanceRiskClass>(
      (highest, descriptor) =>
        RISK_ORDER[descriptor.riskClass] > RISK_ORDER[highest]
          ? descriptor.riskClass
          : highest,
      "NONE",
    );
    const effects = [...new Set(selected.flatMap((descriptor) => descriptor.effectClasses))];
    const arming =
      risk === "HIGH" ? "DESTRUCTIVE" : risk === "NONE" ? "NONE" : "REVERSIBLE";
    return { risk, effects, arming };
  }, [descriptors, sequence.steps]);

  React.useEffect(() => {
    if (selectedKind && available.some((descriptor) => descriptor.kind === selectedKind)) {
      return;
    }
    setSelectedKind(available[0]?.kind ?? "");
  }, [available, selectedKind]);

  React.useEffect(() => {
    if (selectedPreset && presetOptions.includes(selectedPreset)) return;
    setSelectedPreset(presetOptions[0] ?? "");
  }, [presetOptions, selectedPreset]);

  const replaceStep = (index: number, next: MaintenanceActionStep) => {
    const steps = [...sequence.steps];
    steps[index] = next;
    onChange({ ...sequence, steps });
  };
  const remove = (index: number) =>
    onChange({ ...sequence, steps: sequence.steps.filter((_, current) => current !== index) });
  const move = (index: number, offset: -1 | 1) => {
    const next = index + offset;
    if (next < 0 || next >= sequence.steps.length) return;
    const steps = [...sequence.steps];
    [steps[index], steps[next]] = [steps[next], steps[index]];
    onChange({ ...sequence, steps });
  };
  const add = () => {
    const descriptor = available.find((item) => item.kind === selectedKind);
    if (!descriptor) return;
    onChange({ ...sequence, steps: [...sequence.steps, newStep(descriptor, subjectKind)] });
  };
  const loadPreset = () => {
    if (!selectedPreset) return;
    const preset = applyPreset(
      selectedPreset,
      sequence,
      descriptors,
      subjectKind,
      storageRootId,
    );
    if (preset) onChange(preset);
  };

  return (
    <section id="settings-maintenance-action-sequence" className="space-y-3">
      <div>
        <Label className="mb-1 block">{t("settings.maintenanceSequenceTitle")}</Label>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceSequenceHelp")}
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <span className="font-medium text-foreground">
          {t("settings.maintenanceSequenceSummary")}
        </span>
        <span>
          {t("settings.maintenanceSequenceSummaryTarget", {
            target: t(sequenceTargetLabelKey(sequence, subjectKind)),
          })}
        </span>
        <Badge tone={riskClassBadgeTone(summary.risk)}>
          {t("settings.maintenanceSequenceSummaryRisk", {
            risk: t(riskClassLabelKey(summary.risk) ?? summary.risk),
          })}
        </Badge>
        <span>
          {t("settings.maintenanceSequenceSummaryArming", {
            arming: t(effectArmingLabelKey(summary.arming) ?? summary.arming),
          })}
        </span>
        <span>
          {t("settings.maintenanceSequenceSummaryEffects", {
            effects:
              summary.effects.length > 0
                ? summary.effects.map((effect) => effect.replaceAll("_", " ")).join(", ")
                : t("settings.maintenanceSequenceObserveOnly"),
          })}
        </span>
      </div>

      {sequence.steps.length === 0 ? (
        <p className="rounded border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
          {t("settings.maintenanceSequenceEmpty")}
        </p>
      ) : (
        <ol className="space-y-2">
          {sequence.steps.map((step, index) => {
            const descriptor = descriptors.find((item) => item.kind === step.kind);
            if (!descriptor) {
              return (
                <li
                  key={step.id}
                  id={selectorId("settings-maintenance-sequence-step", step.id)}
                  className="flex items-center gap-2 rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3"
                >
                  <div className="min-w-0 flex-1">
                    <code className="text-xs">{step.kind}</code>
                    <p className="mt-1 text-xs text-[var(--scry-warning-text)]">
                      {t("settings.maintenanceSequenceStepUnavailable")}
                    </p>
                  </div>
                  <IconButton
                    type="button"
                    label={t("label.delete")}
                    onClick={() => remove(index)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </IconButton>
                </li>
              );
            }
            return (
              <ActionSequenceStep
                key={step.id}
                step={step}
                descriptor={descriptor}
                index={index}
                count={sequence.steps.length}
                subjectKind={subjectKind}
                qualityProfiles={qualityProfiles}
                update={(next) => replaceStep(index, next)}
                remove={() => remove(index)}
                move={(offset) => move(index, offset)}
              />
            );
          })}
        </ol>
      )}

      <div className="flex flex-wrap items-end gap-2 rounded border border-border bg-muted/30 p-3">
        <div className="min-w-[240px] flex-1">
          <SingleSelectField
            id="settings-maintenance-sequence-preset"
            label={t("settings.maintenanceSequencePreset")}
            placeholder={t("settings.maintenanceSequencePreset")}
            value={selectedPreset}
            onValueChange={(value) =>
              setSelectedPreset(value as MaintenanceActionSequencePreset)
            }
            options={presetOptions.map((preset) => ({
              value: preset,
              label: t(presetLabelKey(preset)),
            }))}
          />
        </div>
        <Button
          type="button"
          variant="secondary"
          disabled={!selectedPreset}
          onClick={loadPreset}
        >
          {t("settings.maintenanceSequencePresetApply")}
        </Button>
      </div>

      <div className="flex flex-wrap items-end gap-2">
        <div className="min-w-[240px] flex-1">
          <SingleSelectField
            id="settings-maintenance-sequence-add"
            label={t("settings.maintenanceSequenceAdd")}
            placeholder={t("settings.maintenanceSequenceAdd")}
            value={selectedKind}
            onValueChange={(value) => setSelectedKind(value as MaintenanceActionStepKind)}
            options={available.map((descriptor) => ({
              value: descriptor.kind,
              label: t(stepKindLabelKey(descriptor.kind)),
            }))}
          />
        </div>
        <Button type="button" variant="secondary" disabled={!selectedKind} onClick={add}>
          <Plus className="mr-2 h-4 w-4" />
          {t("label.add")}
        </Button>
      </div>
    </section>
  );
}
