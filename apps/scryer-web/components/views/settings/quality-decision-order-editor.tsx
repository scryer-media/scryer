import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Gauge,
  Info,
  Lock,
  Plus,
  Trash2,
} from "lucide-react";
import {
  PointerReorderHandle,
  type ReorderDropTarget,
} from "@/components/common/pointer-reorder-handle";
import { InfoHelp } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Label } from "@/components/ui/label";
import {
  UNKNOWN_QUALITY_DECISION_GATE_ICON,
  qualityDecisionGateDescriptor,
  type QualityDecisionGate,
} from "@/lib/constants/quality-decision-gates";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  DECISION_PIPELINE_PRESETS,
  addDecisionGate,
  availableDecisionGates,
  decisionGateLabel,
  decisionPipelineNotices,
  decisionPipelinePresetId,
  describeDecisionPipeline,
  moveDecisionGate,
  removeDecisionGate,
  type DecisionPipelineNotice,
} from "@/lib/utils/quality-decision-pipeline";

const PIPELINE_LIST = "decision-pipeline";
const AVAILABLE_LIST = "decision-available";

const SECTION_CLASS =
  "overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const SECTION_SUMMARY_CLASS =
  "cursor-pointer select-none px-4 py-3 text-sm font-semibold text-[var(--scry-ink2)]";
const SECTION_BODY_CLASS = "border-t border-[var(--scry-border3)] p-4";
const LIST_CLASS =
  "space-y-1.5 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-2";
const ROW_CLASS =
  "flex items-center gap-2 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-card2)] px-2 py-1.5 text-[var(--scry-ink2)]";
const STEP_NUMBER_CLASS =
  "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[11px] font-semibold tabular-nums text-[var(--scry-muted3)]";
const MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

const NOTICE_KEYS: Record<DecisionPipelineNotice, string> = {
  tier_absent: "qualityProfile.decisionWarningTierAbsent",
  tier_below_language: "qualityProfile.decisionNoteTierBelowLanguage",
  revision_absent: "qualityProfile.decisionNoteRevisionAbsent",
  language_needs_library_languages: "qualityProfile.decisionNoteLanguageGates",
};

function GateText({ gate }: { gate: QualityDecisionGate }) {
  const t = useTranslate();
  const descriptor = qualityDecisionGateDescriptor(gate);
  const Icon = descriptor?.icon ?? UNKNOWN_QUALITY_DECISION_GATE_ICON;
  return (
    <>
      <Icon
        className="h-4 w-4 shrink-0 text-[var(--scry-accent-text)]"
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium">
          {decisionGateLabel(gate, t)}
        </span>
        <span className={`block text-xs ${MUTED_TEXT_CLASS}`}>
          {descriptor
            ? t(descriptor.descriptionKey)
            : t("qualityProfile.decisionGateUnknownDescription")}
        </span>
      </span>
    </>
  );
}

/**
 * Ordered comparison steps of a quality profile, ending in the fixed score
 * step. Every drag move is also available through the row buttons.
 */
export function QualityDecisionOrderEditor({
  pipeline,
  onChange,
}: {
  pipeline: QualityDecisionGate[];
  onChange: (pipeline: QualityDecisionGate[]) => void;
}) {
  const t = useTranslate();
  const available = availableDecisionGates(pipeline);
  const activePreset = decisionPipelinePresetId(pipeline);
  const notices = decisionPipelineNotices(pipeline);

  const dropActiveGate = (
    gate: QualityDecisionGate,
    target: ReorderDropTarget,
  ) => {
    if (target.list === AVAILABLE_LIST) {
      onChange(removeDecisionGate(pipeline, gate));
      return;
    }
    if (target.list !== PIPELINE_LIST) return;
    const targetIndex =
      target.item === null
        ? pipeline.length - 1
        : pipeline.indexOf(target.item);
    if (targetIndex >= 0) {
      onChange(moveDecisionGate(pipeline, gate, targetIndex));
    }
  };

  const dropAvailableGate = (
    gate: QualityDecisionGate,
    target: ReorderDropTarget,
  ) => {
    if (target.list !== PIPELINE_LIST) return;
    const targetIndex =
      target.item === null ? undefined : pipeline.indexOf(target.item);
    onChange(
      addDecisionGate(
        pipeline,
        gate,
        targetIndex !== undefined && targetIndex >= 0 ? targetIndex : undefined,
      ),
    );
  };

  return (
    <details
      id={selectorId("settings-quality-profile-decision-order")}
      className={SECTION_CLASS}
      open
    >
      <summary className={SECTION_SUMMARY_CLASS}>
        <span className="inline-flex items-center gap-2">
          {t("qualityProfile.decisionOrder")}
          <InfoHelp
            ariaLabel={t("qualityProfile.decisionOrder")}
            text={t("qualityProfile.decisionOrderInfo")}
          />
        </span>
      </summary>
      <div className={`${SECTION_BODY_CLASS} space-y-4`}>
        <div className="flex flex-wrap items-center gap-2">
          <Label className={`text-xs ${MUTED_TEXT_CLASS}`}>
            {t("qualityProfile.decisionPresets")}
          </Label>
          {DECISION_PIPELINE_PRESETS.map((preset) => {
            const active = activePreset === preset.id;
            return (
              <Button
                key={preset.id}
                id={selectorId(
                  "settings-quality-profile-decision-preset",
                  preset.id,
                )}
                type="button"
                variant="secondary"
                size="sm"
                aria-pressed={active}
                className={cn(
                  active &&
                    "border border-[rgba(var(--scry-accent-rgb),0.45)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)] hover:bg-[rgba(var(--scry-accent-rgb),0.2)]",
                )}
                onClick={() => onChange([...preset.pipeline])}
              >
                {t(preset.labelKey)}
              </Button>
            );
          })}
        </div>

        <p
          id={selectorId("settings-quality-profile-decision-summary")}
          className="text-sm text-[var(--scry-ink2)]"
          aria-live="polite"
        >
          {describeDecisionPipeline(pipeline, t)}
        </p>

        <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,20rem)]">
          <ol className={LIST_CLASS} data-reorder-list={PIPELINE_LIST}>
            {pipeline.map((gate, index) => {
              const label = decisionGateLabel(gate, t);
              return (
                <li
                  key={gate}
                  id={selectorId(
                    "settings-quality-profile-decision-step",
                    gate,
                  )}
                  className={ROW_CLASS}
                  data-reorder-item={gate}
                >
                  <span className={STEP_NUMBER_CLASS}>{index + 1}</span>
                  <PointerReorderHandle
                    id={selectorId(
                      "settings-quality-profile-decision-step-drag",
                      gate,
                    )}
                    className={MUTED_TEXT_CLASS}
                    title={t("qualityProfile.decisionStepDrag", {
                      value: label,
                    })}
                    onDrop={(target) => dropActiveGate(gate, target)}
                  />
                  <GateText gate={gate} />
                  <div className="flex shrink-0 items-center gap-1">
                    <IconButton
                      id={selectorId(
                        "settings-quality-profile-decision-step-up",
                        gate,
                      )}
                      tone="reorder"
                      disabled={index === 0}
                      label={t("qualityProfile.decisionStepMoveUp", {
                        value: label,
                      })}
                      onClick={() =>
                        onChange(moveDecisionGate(pipeline, gate, index - 1))
                      }
                    >
                      <ArrowUp className="h-4 w-4" aria-hidden="true" />
                    </IconButton>
                    <IconButton
                      id={selectorId(
                        "settings-quality-profile-decision-step-down",
                        gate,
                      )}
                      tone="reorder"
                      disabled={index === pipeline.length - 1}
                      label={t("qualityProfile.decisionStepMoveDown", {
                        value: label,
                      })}
                      onClick={() =>
                        onChange(moveDecisionGate(pipeline, gate, index + 1))
                      }
                    >
                      <ArrowDown className="h-4 w-4" aria-hidden="true" />
                    </IconButton>
                    <IconButton
                      id={selectorId(
                        "settings-quality-profile-decision-step-remove",
                        gate,
                      )}
                      tone="delete"
                      label={t("qualityProfile.decisionStepRemove", {
                        value: label,
                      })}
                      onClick={() =>
                        onChange(removeDecisionGate(pipeline, gate))
                      }
                    >
                      <Trash2 className="h-4 w-4" aria-hidden="true" />
                    </IconButton>
                  </div>
                </li>
              );
            })}
            <li
              id={selectorId("settings-quality-profile-decision-step-score")}
              className={cn(ROW_CLASS, "border-dashed bg-transparent")}
            >
              <span className={STEP_NUMBER_CLASS}>{pipeline.length + 1}</span>
              <Lock
                className={`h-4 w-4 shrink-0 ${MUTED_TEXT_CLASS}`}
                aria-label={t("qualityProfile.decisionScoreLocked")}
              />
              <Gauge
                className="h-4 w-4 shrink-0 text-[var(--scry-accent-text)]"
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium">
                  {t("qualityProfile.decisionGateScore")}
                </span>
                <span className={`block text-xs ${MUTED_TEXT_CLASS}`}>
                  {t("qualityProfile.decisionGateScoreDescription")}
                </span>
              </span>
              <span className="shrink-0 rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-[10px] text-[var(--scry-muted3)]">
                {t("qualityProfile.decisionScoreLocked")}
              </span>
            </li>
          </ol>

          <div>
            <Label className="mb-2 block text-[var(--scry-ink2)]">
              {t("qualityProfile.decisionAvailableSteps")}
            </Label>
            <ul
              className={LIST_CLASS}
              data-reorder-list={AVAILABLE_LIST}
              data-reorder-unordered
            >
              {available.length === 0 ? (
                <li className={`px-1 py-1 text-xs ${MUTED_TEXT_CLASS}`}>
                  {t("qualityProfile.decisionAllStepsInUse")}
                </li>
              ) : (
                available.map((gate) => {
                  const label = decisionGateLabel(gate, t);
                  return (
                    <li
                      key={gate}
                      id={selectorId(
                        "settings-quality-profile-decision-available",
                        gate,
                      )}
                      className={ROW_CLASS}
                      data-reorder-item={gate}
                    >
                      <PointerReorderHandle
                        id={selectorId(
                          "settings-quality-profile-decision-available-drag",
                          gate,
                        )}
                        className={MUTED_TEXT_CLASS}
                        title={t("qualityProfile.decisionStepAdd", {
                          value: label,
                        })}
                        onDrop={(target) => dropAvailableGate(gate, target)}
                      />
                      <GateText gate={gate} />
                      <IconButton
                        id={selectorId(
                          "settings-quality-profile-decision-add",
                          gate,
                        )}
                        tone="auto"
                        className="shrink-0"
                        label={t("qualityProfile.decisionStepAdd", {
                          value: label,
                        })}
                        onClick={() =>
                          onChange(addDecisionGate(pipeline, gate))
                        }
                      >
                        <Plus className="h-4 w-4" aria-hidden="true" />
                      </IconButton>
                    </li>
                  );
                })
              )}
            </ul>
          </div>
        </div>

        {notices.length > 0 ? (
          <ul className="space-y-2">
            {notices.map((notice) => {
              const warning = notice === "tier_absent";
              const NoticeIcon = warning ? AlertTriangle : Info;
              return (
                <li
                  key={notice}
                  id={selectorId(
                    "settings-quality-profile-decision-notice",
                    notice,
                  )}
                  role={warning ? "alert" : undefined}
                  className={cn(
                    "flex items-start gap-2 rounded-[10px] border px-3 py-2 text-xs",
                    warning
                      ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
                      : "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
                  )}
                >
                  <NoticeIcon
                    className="mt-0.5 h-3.5 w-3.5 shrink-0"
                    aria-hidden="true"
                  />
                  <span>{t(NOTICE_KEYS[notice])}</span>
                </li>
              );
            })}
          </ul>
        ) : null}
      </div>
    </details>
  );
}
