import * as React from "react";

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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { useRuleOutputDescriptors } from "@/lib/hooks/use-rule-builder-schema";
import type { DashboardPanelKind } from "@/lib/utils/dashboard-layout";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  LIFECYCLE_UPCOMING_ACTION_GROUPS,
  LIFECYCLE_UPCOMING_NAME_MAX_LENGTH,
  lifecycleUpcomingGroupHead,
  lifecycleUpcomingPanelSettings,
  lifecycleUpcomingSettingsValue,
  type LifecycleUpcomingActionGroup,
} from "@/lib/utils/lifecycle-upcoming-panel";
import { ruleOutputHeadLabel } from "@/lib/utils/rule-output-vocabulary";

type PanelSettingsDialogArgs = {
  panelId: string;
  /** The placement's settings as they stand in the draft. */
  settings: unknown;
  /** Writes into the draft only; nothing is saved until the layout is. */
  onApply: (settings: Record<string, unknown> | null) => void;
  onClose: () => void;
};

type ConfigurablePanel = {
  /** The name the placement was given, or "" to use the kind's title. */
  customName: (settings: unknown) => string;
  renderDialog: (args: PanelSettingsDialogArgs) => React.ReactNode;
};

/**
 * The panel kinds a placement can configure while the dashboard is being
 * edited, and the dialog each opens. Kinds not listed have no settings.
 */
export const CONFIGURABLE_DASHBOARD_PANELS: Readonly<
  Partial<Record<DashboardPanelKind, ConfigurablePanel>>
> = {
  "lifecycle-upcoming": {
    customName: (settings) => lifecycleUpcomingPanelSettings(settings).name,
    renderDialog: (args) => (
      <LifecycleUpcomingSettingsDialog key={args.panelId} {...args} />
    ),
  },
};

function LifecycleUpcomingSettingsDialog({
  panelId,
  settings: saved,
  onApply,
  onClose,
}: PanelSettingsDialogArgs) {
  const t = useTranslate();
  const descriptors = useRuleOutputDescriptors("lifecycle");
  const initial = React.useMemo(
    () => lifecycleUpcomingPanelSettings(saved),
    [saved],
  );
  const [name, setName] = React.useState(initial.name);
  const [actions, setActions] = React.useState<
    ReadonlySet<LifecycleUpcomingActionGroup>
  >(() => new Set(initial.actions));
  const idPrefix = selectorId("dashboard-lifecycle-upcoming-settings", panelId);
  const panelTitle = t("dashboard.lifecycleUpcoming");

  const toggle = (group: LifecycleUpcomingActionGroup, checked: boolean) =>
    setActions((current) => {
      const next = new Set(current);
      if (checked) {
        next.add(group);
      } else {
        next.delete(group);
      }
      return next;
    });

  const apply = (event: React.FormEvent) => {
    event.preventDefault();
    onApply(
      lifecycleUpcomingSettingsValue({
        actions: LIFECYCLE_UPCOMING_ACTION_GROUPS.filter((group) =>
          actions.has(group),
        ),
        name,
      }),
    );
    onClose();
  };

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent
        id={idPrefix}
        className="max-h-[90vh] overflow-y-auto sm:max-w-lg"
      >
        <form className="min-w-0 space-y-4" onSubmit={apply}>
          <DialogHeader>
            <DialogTitle>
              {t("dashboard.layout.configurePanel", {
                name: initial.name || panelTitle,
              })}
            </DialogTitle>
            <DialogDescription>
              {t("dashboard.lifecycleUpcomingSettingsDescription")}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor={selectorId(idPrefix, "name")}>
              {t("dashboard.lifecycleUpcomingName")}
            </Label>
            <Input
              id={selectorId(idPrefix, "name")}
              value={name}
              maxLength={LIFECYCLE_UPCOMING_NAME_MAX_LENGTH}
              placeholder={panelTitle}
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          <fieldset className="min-w-0 space-y-3">
            <legend className="mb-2 text-sm font-medium text-[var(--scry-ink3)]">
              {t("label.actions")}
            </legend>
            <CheckboxField
              id={selectorId(idPrefix, "all")}
              label={t("dashboard.lifecycleUpcomingAllActions")}
              checked={actions.size === 0}
              onCheckedChange={(checked) => {
                if (checked === true) {
                  setActions(new Set());
                }
              }}
            />
            <div className="grid gap-3 sm:grid-cols-2">
              {LIFECYCLE_UPCOMING_ACTION_GROUPS.map((group) => (
                <CheckboxField
                  key={group}
                  id={selectorId(idPrefix, "group", group)}
                  label={ruleOutputHeadLabel(
                    t,
                    descriptors,
                    lifecycleUpcomingGroupHead(group),
                  )}
                  checked={actions.has(group)}
                  onCheckedChange={(checked) => toggle(group, checked === true)}
                />
              ))}
            </div>
          </fieldset>

          <DialogFooter>
            <Button
              id={selectorId(idPrefix, "cancel")}
              type="button"
              variant="secondary"
              onClick={onClose}
            >
              {t("label.cancel")}
            </Button>
            <Button id={selectorId(idPrefix, "apply")} type="submit">
              {t("dashboard.layout.applyPanelSettings")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
