import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronDown, ChevronUp, Power, PowerOff } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { DownloadClientRecord, DownloadClientRoutingSettings } from "@/lib/types";
import { cn } from "@/lib/utils";
import { DOWNLOAD_CLIENT_ROUTING_EMPTY } from "@/lib/constants/nzbget";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

type ScopeRoutingRecord = Record<string, DownloadClientRoutingSettings>;

function DownloadClientRoutingActionButton({
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

export const NzbgetIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox="182.47 40.33 528 528"
    fill="none"
    {...props}
  >
    <ellipse cx="446.47417" cy="304.33312" rx="264" ry="263.99999" fill="#fafafa" />
    <ellipse cx="445.47418" cy="304.99977" rx="239.8589" ry="239.66666" fill="#333733" />
    <ellipse cx="445.33311" cy="303.33311" rx="226" ry="226" fill="#37d134" />
    <path
      d="m330.34323,434.81804l116.49998,-116.66662l116.49998,116.66662l-232.99996,0z"
      fill="#000000"
    />
    <path
      d="m330.34323,273.48469l116.49998,116.66662l116.49998,-116.66662l-232.99996,0z"
      fill="#000000"
      transform="rotate(-180 446.843 376.485)"
    />
    <rect x="398.66641" y="266.66647" width="94.66664" height="51.33332" fill="#000000" />
    <path d="m399.33309,215.33316l92.66665,0l0,33.33332l-92.66665,0l0,-33.33332z" fill="#000000" />
    <path d="m399.33309,163.99984l92.66664,0l0,33.33332l-92.66664,0l0,-33.33332z" fill="#000000" />
  </svg>
);

export const QBitTorrentIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox="0 0 1024 1024"
    fill="none"
    {...props}
  >
    <circle
      cx="512"
      cy="512"
      r="496"
      fill="#72b4f5"
      stroke="#daefff"
      strokeWidth="32"
    />
    <path
      d="M712.9 332.4c44.4 0 78.9 15.2 103.4 45.7 24.7 30.2 37 73.1 37 128.7 0 55.5-12.4 98.8-37.3 129.6-24.7 30.7-59 46-103.1 46-22 0-42.2-4-60.5-12-18.1-8.2-33.3-20.8-45.7-37.6H603l-10.8 43.5h-36.7V196h51.2v116.6c0 26.1-.8 49.6-2.5 70.4h2.5c23.9-33.7 59.3-50.6 106.2-50.6m-7.4 42.9c-35 0-60.2 10.1-75.6 30.2-15.4 20-23.1 53.7-23.1 101.2s7.9 81.6 23.8 102.1c15.8 20.4 41.2 30.5 76.2 30.5 31.5 0 54.9-11.4 70.4-34.3 15.4-23 23.1-56.1 23.1-99.1q0-66-23.1-98.4c-15.5-21.4-39.4-32.2-71.7-32.2"
      fill="#ffffff"
    />
    <path
      d="M317.3 639.5c34.2 0 59-9.2 74.7-27.5 15.6-18.3 24-49.2 25-92.6V508c0-47.3-8-81.4-24.1-102.1-16-20.8-41.5-31.2-76.2-31.2-30 0-53.1 11.7-69.1 35.2-15.8 23.2-23.8 56.2-23.8 98.8s7.8 75.1 23.5 97.5c15.8 22.1 39.1 33.2 70 33.3m-7.7 42.8c-43.6 0-77.7-15.3-102.1-46-24.5-30.7-36.7-73.4-36.7-128.4 0-55.3 12.3-98.5 37-129.6s59-46.6 103.1-46.6q69.45 0 106.8 52.5h2.8l7.4-46.3h40.4v490h-51.2V683.3c0-20.6 1.1-38.1 3.4-52.5h-4c-23.8 34.4-59.4 51.5-106.9 51.5"
      fill="#c8e8ff"
    />
  </svg>
);

export const SabnzbdIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000" fill="none" {...props}>
    <path
      fill="none"
      stroke="#f5f5f5"
      strokeLinejoin="round"
      strokeWidth="74"
      d="M200.4 39.3h598.1v437.8h161l-460.1 483L39.4 477h161z"
    />
    <path fill="#ffb300" fillRule="evenodd" d="M200.4 39.3h598.1v437.8h161l-460.1 483-460-483h161z" />
    <path fill="#ffca28" fillRule="evenodd" d="M499.4 960.2 201.1 39.4h596.7z" />
    <path
      fill="none"
      stroke="#f5f5f5"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="74"
      d="M329.2 843.5H83v-51.8h146.1v-45.9H83V596.9h246.2v51.5H183.1v45.9h146.1zm292.2 0H375.2V694.3h146.1v-45.9H375.2v-51.5h246.2zm-146.1-97.8h46v46h-46zm192.1 97.8v-344h100.1v97.4h146.1v246.6zm100.1-195.2h46v143.4h-46z"
    />
    <path
      fill="#0f0f0f"
      fillRule="evenodd"
      d="M329.2 843.5H83v-51.8h146.1v-45.9H83V596.9h246.2v51.5H183.1v45.9h146.1zm292.2 0H375.2V694.3h146.1v-45.9H375.2v-51.5h246.2zm-146.1-51.8h46v-46h-46zm192.1 51.9v-344h100.1V597h146.1v246.6zm100.1-51.9h46V648.4h-46z"
    />
  </svg>
);

export const WeaverIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" {...props}>
    <rect x="8" y="10" width="48" height="44" rx="12" fill="#141c2b" />
    <path
      d="M18 20l8 24 6-18 6 18 8-24"
      stroke="#78f0c5"
      strokeWidth="5"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

type DownloadClientTypeOption = {
  value: string;
  iconSrc?: string;
  icon: (props: React.ComponentPropsWithoutRef<"svg">) => React.JSX.Element;
};

const DOWNLOAD_CLIENT_TYPE_OPTIONS: DownloadClientTypeOption[] = [
  {
    value: "nzbget",
    icon: NzbgetIcon,
  },
  {
    value: "sabnzbd",
    icon: SabnzbdIcon,
  },
  {
    value: "weaver",
    iconSrc: "download-clients/weaver.webp",
    icon: WeaverIcon,
  },
  {
    value: "qbittorrent",
    icon: QBitTorrentIcon,
  },
];

function getDownloadClientTypeOption(typeValue: string) {
  const normalizedType = typeValue.trim().toLowerCase();
  return (
    DOWNLOAD_CLIENT_TYPE_OPTIONS.find((option) => option.value === normalizedType) ??
    DOWNLOAD_CLIENT_TYPE_OPTIONS[0]
  );
}

export function DownloadClientTypeLogo({
  typeValue,
  className = "h-4 w-4",
}: {
  typeValue: string;
  className?: string;
}) {
  const option = getDownloadClientTypeOption(typeValue);
  const FallbackIcon = option.icon;
  const [failedToLoadImage, setFailedToLoadImage] = React.useState(false);

  if (failedToLoadImage || !option.iconSrc) {
    return <FallbackIcon className={`${className} object-contain`} aria-hidden="true" role="img" />;
  }

  return (
    <img
      src={option.iconSrc}
      alt=""
      className={`${className} object-contain`}
      role="img"
      onError={() => setFailedToLoadImage(true)}
    />
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

const PRIORITY_VALUES = new Set(DOWNLOAD_PRIORITY_OPTIONS.map((item) => item.value));

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
  moveDownloadClientInScope: (clientId: string, direction: "up" | "down") => void;
};

export const DownloadClientRoutingPanel = React.memo(function DownloadClientRoutingPanel({
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
  const clientById = React.useMemo(
    () => Object.fromEntries(downloadClients.map((client) => [client.id, client])),
    [downloadClients],
  );

  const orderedDownloadClientIds = React.useMemo(() => {
    const configuredIds = activeScopeRoutingOrder.filter((clientId) => clientById[clientId]);
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
    <Card>
      <CardHeader>
        <CardTitle>
          {t("settings.downloadClientRoutingScope", {
            scope: scopeLabel,
          })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="overflow-x-auto rounded border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.downloadClientPriority")}</TableHead>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("label.type")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRoutingGloballyEnabled")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRoutingEnabled")}</TableHead>
                <TableHead>{t("settings.downloadClientCategory")}</TableHead>
                <TableHead>{t("settings.downloadClientRecentPriority")}</TableHead>
                <TableHead>{t("settings.downloadClientOlderPriority")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRemoveCompleted")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRemoveFailed")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orderedDownloadClientIds.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={11} className="text-muted-foreground">
                    {t("settings.noDownloadClientsFound")}
                  </TableCell>
                </TableRow>
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

                  return (
                    <TableRow key={client.id}>
                      <TableCell>{index + 1}</TableCell>
                      <TableCell>{client.name}</TableCell>
                      <TableCell className="text-center">
                        <span className="inline-flex items-center justify-center">
                          <DownloadClientTypeLogo typeValue={client.clientType} />
                          <span className="sr-only">{client.clientType}</span>
                        </span>
                      </TableCell>
                      <TableCell className="text-center align-middle">
                        <RenderBooleanIcon
                          value={client.isEnabled}
                          label={`${t("settings.downloadClientRoutingGloballyEnabled")}: ${client.name}`}
                        />
                      </TableCell>
                      <TableCell className="text-center align-middle">
                        <RenderBooleanIcon
                          value={client.isEnabled && routing.enabled}
                          label={`${t("settings.downloadClientRoutingEnabled")}: ${client.name}`}
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          value={routing.category}
                          onChange={(event) =>
                            handleRoutingCategoryChange(client.id, event.target.value)
                          }
                          onBlur={(event) =>
                            handleRoutingCategoryBlur(client.id, event.target.value)
                          }
                          disabled={controlsDisabled}
                          placeholder={t("settings.downloadClientCategoryPlaceholder")}
                        />
                      </TableCell>
                      <TableCell>
                        <Select
                          value={normalizePriorityValue(routing.recentQueuePriority)}
                          onValueChange={(value) =>
                            handleRoutingRecentPriorityChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger className="w-full">
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
                      </TableCell>
                      <TableCell>
                        <Select
                          value={normalizePriorityValue(routing.olderQueuePriority)}
                          onValueChange={(value) =>
                            handleRoutingOlderPriorityChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger className="w-full">
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
                      </TableCell>
                      <TableCell className="text-center">
                        <Checkbox
                          checked={routing.removeCompleted}
                          onCheckedChange={(checked) =>
                            handleRoutingRemoveCompletedChange(client.id, checked === true)
                          }
                          disabled={controlsDisabled}
                        />
                      </TableCell>
                      <TableCell className="text-center">
                        <Checkbox
                          checked={routing.removeFailed}
                          onCheckedChange={(checked) =>
                            handleRoutingRemoveFailedChange(client.id, checked === true)
                          }
                          disabled={controlsDisabled}
                        />
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-2">
                          <DownloadClientRoutingActionButton
                            tone={routing.enabled ? "disabled" : "enabled"}
                            label={
                              routing.enabled
                                ? t("label.disable")
                                : t("label.enable")
                            }
                            onClick={() =>
                              void updateDownloadClientRoutingForScope(client.id, {
                                enabled: !routing.enabled,
                              })
                            }
                            disabled={controlsDisabled || !client.isEnabled}
                          >
                            {routing.enabled ? (
                              <PowerOff className="h-4 w-4" />
                            ) : (
                              <Power className="h-4 w-4" />
                            )}
                          </DownloadClientRoutingActionButton>
                          <DownloadClientRoutingActionButton
                            tone="reorder"
                            label={`${t("label.moveUp")} ${client.name}`}
                            onClick={() => moveClientUp(client.id)}
                            disabled={controlsDisabled || index === 0}
                          >
                            <ChevronUp className="h-4 w-4" />
                          </DownloadClientRoutingActionButton>
                          <DownloadClientRoutingActionButton
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
  );
});
