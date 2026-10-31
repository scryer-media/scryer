import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { IconButton } from "@/components/ui/icon-button";
import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronDown, ChevronUp, Download } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  DownloadClientRecord,
  DownloadClientRoutingSettings,
} from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
import { DOWNLOAD_CLIENT_ROUTING_EMPTY } from "@/lib/constants/nzbget";
import { type BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import { useSeedingProfileOptions } from "@/lib/hooks/use-seeding-profile-options";
import {
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
} from "@/lib/utils/seeding-profiles";

type ScopeRoutingRecord = Record<string, DownloadClientRoutingSettings>;

function DownloadClientRoutingActionButton({
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

export const DOWNLOAD_PRIORITY_OPTIONS = [
  { value: "force", label: "settings.downloadClientPriorityForce" },
  { value: "very high", label: "settings.downloadClientPriorityVeryHigh" },
  { value: "high", label: "settings.downloadClientPriorityHigh" },
  { value: "normal", label: "settings.downloadClientPriorityNormal" },
  { value: "low", label: "settings.downloadClientPriorityLow" },
  { value: "very low", label: "settings.downloadClientPriorityVeryLow" },
];

const PRIORITY_VALUES = new Set(
  DOWNLOAD_PRIORITY_OPTIONS.map((item) => item.value),
);

function normalizePriorityValue(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
}

function normalizePriorityValueForSave(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
}

function supportsRoutingSeedingProfile(clientType: string): boolean {
  return ["qbittorrent", "deluge", "transmission", "rtorrent"].includes(
    clientType.trim().toLowerCase(),
  );
}

function RoutingField({
  htmlFor,
  label,
  children,
}: {
  htmlFor: string;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="min-w-0 space-y-1.5">
      <Label htmlFor={htmlFor} className="text-xs text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  );
}

type DownloadClientRoutingPanelProps = {
  scopeLabel: string;
  downloadClients: DownloadClientRecord[];
  activeScopeRouting: ScopeRoutingRecord;
  activeScopeRoutingOrder: string[];
  downloadClientRoutingLoading: boolean;
  downloadClientRoutingSaving: boolean;
  updateDownloadClientRoutingForScope: (
    clientId: string,
    nextValue: Partial<DownloadClientRoutingSettings>,
    options?: { save?: boolean },
  ) => Promise<void> | void;
  moveDownloadClientInScope: (
    clientId: string,
    direction: "up" | "down",
  ) => void;
};

export const DownloadClientRoutingPanel = React.memo(
  function DownloadClientRoutingPanel({
    scopeLabel,
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
  }: DownloadClientRoutingPanelProps) {
    const t = useTranslate();
    // Seeding profiles are reference data for this table, not scope state, so the
    // panel reads them itself instead of threading an identical prop through the
    // two unrelated parents that render it.
    const { options: seedingProfileOptions } = useSeedingProfileOptions();
    const clientById = React.useMemo(
      () =>
        Object.fromEntries(
          downloadClients.map((client) => [client.id, client]),
        ),
      [downloadClients],
    );

    const orderedDownloadClientIds = React.useMemo(() => {
      const configuredIds = activeScopeRoutingOrder.filter(
        (clientId) => clientById[clientId],
      );
      const configuredIdSet = new Set(configuredIds);
      const missingIds = downloadClients
        .map((client) => client.id)
        .filter((clientId) => !configuredIdSet.has(clientId));
      return [...configuredIds, ...missingIds];
    }, [activeScopeRoutingOrder, clientById, downloadClients]);

    const handleRoutingCategoryChange = React.useCallback(
      (clientId: string, value: string) => {
        void updateDownloadClientRoutingForScope(
          clientId,
          {
            category: value,
          },
          { save: false },
        );
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingCategoryBlur = React.useCallback(
      (clientId: string, value: string) => {
        void updateDownloadClientRoutingForScope(clientId, {
          category: value,
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingRecentPriorityChange = React.useCallback(
      (clientId: string, value: string) => {
        void updateDownloadClientRoutingForScope(clientId, {
          recentQueuePriority: normalizePriorityValueForSave(value),
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingOlderPriorityChange = React.useCallback(
      (clientId: string, value: string) => {
        void updateDownloadClientRoutingForScope(clientId, {
          olderQueuePriority: normalizePriorityValueForSave(value),
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingRemoveCompletedChange = React.useCallback(
      (clientId: string, checked: boolean) => {
        void updateDownloadClientRoutingForScope(clientId, {
          removeCompleted: checked,
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingRemoveFailedChange = React.useCallback(
      (clientId: string, checked: boolean) => {
        void updateDownloadClientRoutingForScope(clientId, {
          removeFailed: checked,
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const handleRoutingSeedingProfileChange = React.useCallback(
      (clientId: string, value: string) => {
        void updateDownloadClientRoutingForScope(clientId, {
          seedingProfileId: seedingProfileSelectValueToId(value),
        });
      },
      [updateDownloadClientRoutingForScope],
    );

    const moveClientUp = React.useCallback(
      (clientId: string) => {
        moveDownloadClientInScope(clientId, "up");
      },
      [moveDownloadClientInScope],
    );

    const moveClientDown = React.useCallback(
      (clientId: string) => {
        moveDownloadClientInScope(clientId, "down");
      },
      [moveDownloadClientInScope],
    );

    return (
      <section
        id="download-client-routing-panel"
        className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6"
      >
        <div className="flex items-center gap-2.5">
          <Download className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
          <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">
            {t("settings.downloadClientRoutingScope", {
              scope: scopeLabel,
            })}
          </h2>
        </div>
        <div className="mt-5 space-y-3">
          {orderedDownloadClientIds.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("settings.noDownloadClientsFound")}
            </p>
          ) : (
            orderedDownloadClientIds.map((clientId, index) => {
              const client = clientById[clientId];
              if (!client) {
                return null;
              }
              const routing =
                activeScopeRouting[client.id] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
              const controlsDisabled =
                downloadClientRoutingLoading || downloadClientRoutingSaving;
              const supportsSeedingProfile = supportsRoutingSeedingProfile(
                client.clientType,
              );
              const categoryId = selectorId(
                "download-client-routing-category",
                client.name,
              );
              const recentPriorityId = selectorId(
                "download-client-routing-recent-priority",
                client.name,
              );
              const olderPriorityId = selectorId(
                "download-client-routing-older-priority",
                client.name,
              );
              const seedingProfileId = selectorId(
                "download-client-routing-seeding-profile",
                client.name,
              );
              const enabledId = selectorId(
                "download-client-routing-enabled",
                client.name,
              );
              const removeCompletedId = selectorId(
                "download-client-routing-remove-completed",
                client.name,
              );
              const removeFailedId = selectorId(
                "download-client-routing-remove-failed",
                client.name,
              );

              return (
                <div
                  key={client.id}
                  id={selectorId("download-client-routing-row", client.name)}
                  data-ui="settings-table-row"
                  className="@container rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 sm:p-4"
                >
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                    <div className="flex items-center gap-1">
                      <span className="w-5 text-center text-sm tabular-nums text-muted-foreground">
                        {index + 1}
                      </span>
                      <DownloadClientRoutingActionButton
                        id={selectorId(
                          "download-client-routing-move-up",
                          client.name,
                        )}
                        tone="reorder"
                        label={`${t("label.moveUp")} ${client.name}`}
                        onClick={() => moveClientUp(client.id)}
                        disabled={controlsDisabled || index === 0}
                      >
                        <ChevronUp className="h-4 w-4" />
                      </DownloadClientRoutingActionButton>
                      <DownloadClientRoutingActionButton
                        id={selectorId(
                          "download-client-routing-move-down",
                          client.name,
                        )}
                        tone="reorder"
                        label={`${t("label.moveDown")} ${client.name}`}
                        onClick={() => moveClientDown(client.id)}
                        disabled={
                          controlsDisabled ||
                          index >= orderedDownloadClientIds.length - 1
                        }
                      >
                        <ChevronDown className="h-4 w-4" />
                      </DownloadClientRoutingActionButton>
                    </div>
                    <div className="flex min-w-0 flex-1 items-center gap-2">
                      <DownloadClientTypeLogo typeValue={client.clientType} />
                      <span className="sr-only">{client.clientType}</span>
                      <span className="truncate font-semibold text-[var(--scry-ink2)]">
                        {client.name}
                      </span>
                      {!client.isEnabled ? (
                        <span
                          className="shrink-0 rounded-full border border-[var(--scry-border2)] px-2 py-0.5 text-xs text-muted-foreground"
                          title={`${t("settings.downloadClientRoutingGloballyEnabled")}: ${t("label.no")}`}
                        >
                          {t("label.disabled")}
                        </span>
                      ) : null}
                    </div>
                    <label
                      htmlFor={enabledId}
                      className="flex cursor-pointer items-center gap-2 text-sm font-medium"
                    >
                      {t("settings.downloadClientRoutingEnabled")}
                      <Checkbox
                        id={enabledId}
                        size="large"
                        checked={client.isEnabled && routing.enabled}
                        disabled={controlsDisabled || !client.isEnabled}
                        onCheckedChange={(checked) =>
                          void updateDownloadClientRoutingForScope(client.id, {
                            enabled: checked === true,
                          })
                        }
                      />
                    </label>
                  </div>
                  <div className="mt-3 grid grid-cols-1 gap-3 @md:grid-cols-2 @4xl:grid-cols-4">
                    <RoutingField
                      htmlFor={categoryId}
                      label={t("settings.downloadClientCategory")}
                    >
                      <Input
                        id={categoryId}
                        value={routing.category}
                        onChange={(event) =>
                          handleRoutingCategoryChange(
                            client.id,
                            event.target.value,
                          )
                        }
                        onBlur={(event) =>
                          handleRoutingCategoryBlur(
                            client.id,
                            event.target.value,
                          )
                        }
                        disabled={controlsDisabled}
                        placeholder={t(
                          "settings.downloadClientCategoryPlaceholder",
                        )}
                      />
                    </RoutingField>
                    <RoutingField
                      htmlFor={recentPriorityId}
                      label={t("settings.downloadClientRecentPriority")}
                    >
                      <Select
                        value={normalizePriorityValue(
                          routing.recentQueuePriority,
                        )}
                        onValueChange={(value) =>
                          handleRoutingRecentPriorityChange(client.id, value)
                        }
                        disabled={controlsDisabled}
                      >
                        <SelectTrigger id={recentPriorityId} className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.label)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </RoutingField>
                    <RoutingField
                      htmlFor={olderPriorityId}
                      label={t("settings.downloadClientOlderPriority")}
                    >
                      <Select
                        value={normalizePriorityValue(
                          routing.olderQueuePriority,
                        )}
                        onValueChange={(value) =>
                          handleRoutingOlderPriorityChange(client.id, value)
                        }
                        disabled={controlsDisabled}
                      >
                        <SelectTrigger id={olderPriorityId} className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.label)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </RoutingField>
                    {supportsSeedingProfile ? (
                      <RoutingField
                        htmlFor={seedingProfileId}
                        label={t("settings.seedingProfileColumn")}
                      >
                        <Select
                          value={seedingProfileSelectValue(
                            routing.seedingProfileId,
                          )}
                          onValueChange={(value) =>
                            handleRoutingSeedingProfileChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger
                            id={seedingProfileId}
                            className="w-full"
                            aria-label={t(
                              "settings.seedingProfileRoutingLabel",
                              {
                                name: client.name,
                              },
                            )}
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={SEEDING_PROFILE_INHERIT_VALUE}>
                              {t("settings.seedingProfileRoutingInherit")}
                            </SelectItem>
                            {routing.seedingProfileId &&
                            !seedingProfileOptions.some(
                              (option) =>
                                option.id === routing.seedingProfileId,
                            ) ? (
                              <SelectItem value={routing.seedingProfileId}>
                                {t("settings.seedingProfileMissing", {
                                  id: routing.seedingProfileId,
                                })}
                              </SelectItem>
                            ) : null}
                            {seedingProfileOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </RoutingField>
                    ) : null}
                  </div>
                  <div className="mt-3 flex flex-wrap gap-x-6 gap-y-2">
                    <label
                      htmlFor={removeCompletedId}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <Checkbox
                        id={removeCompletedId}
                        checked={routing.removeCompleted}
                        onCheckedChange={(checked) =>
                          handleRoutingRemoveCompletedChange(
                            client.id,
                            checked === true,
                          )
                        }
                        disabled={controlsDisabled}
                      />
                      {t("settings.downloadClientRemoveCompleted")}
                    </label>
                    <label
                      htmlFor={removeFailedId}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <Checkbox
                        id={removeFailedId}
                        checked={routing.removeFailed}
                        onCheckedChange={(checked) =>
                          handleRoutingRemoveFailedChange(
                            client.id,
                            checked === true,
                          )
                        }
                        disabled={controlsDisabled}
                      />
                      {t("settings.downloadClientRemoveFailed")}
                    </label>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </section>
    );
  },
);
