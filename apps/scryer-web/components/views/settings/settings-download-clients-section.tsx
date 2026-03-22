
import * as React from "react";
import { ChevronDown, ChevronUp, Edit, Power, PowerOff, Server, Trash2 } from "lucide-react";
import { InfoHelp } from "@/components/common/info-help";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  buildWeaverApiKeyUrl,
  buildUrlPreview,
} from "@/lib/utils/download-clients";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { DownloadClientRecord, DownloadClientDraft, DownloadClientTypeOption } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

type DownloadClientTypeLogoOption = {
  value: string;
  iconSrc?: string;
  icon?: (props: React.ComponentPropsWithoutRef<"svg">) => React.JSX.Element;
};

const NzbgetIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
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
      transform="rotate(-180 446.843 376.485)"
    />
    <rect x="398.66641" y="266.66647" width="94.66664" height="51.33332" fill="#000000" />
    <path d="m399.33309,215.33316l92.66665,0l0,33.33332l-92.66665,0l0,-33.33332z" fill="#000000" />
    <path d="m399.33309,163.99984l92.66664,0l0,33.33332l-92.66664,0l0,-33.33332z" fill="#000000" />
  </svg>
);

const QBitTorrentIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
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

const SabnzbdIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
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

const WeaverIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
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

function DownloadClientActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "enabled" | "disabled" | "delete">;
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

export type SettingsDownloadClientsSectionProps = {
  editingDownloadClientId: string | null;
  downloadClientTypeOptions: DownloadClientTypeOption[];
  downloadClientDraft: DownloadClientDraft;
  setDownloadClientDraft: React.Dispatch<React.SetStateAction<DownloadClientDraft>>;
  submitDownloadClient: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  testDownloadClientConnection: () => Promise<void>;
  isTestingDownloadClientConnection: boolean;
  mutatingDownloadClientId: string | null;
  resetDownloadClientDraft: () => void;
  settingsDownloadClients: DownloadClientRecord[];
  editDownloadClient: (downloadClient: DownloadClientRecord) => void;
  toggleDownloadClientEnabled: (downloadClient: DownloadClientRecord) => Promise<void>;
  deleteDownloadClient: (downloadClient: DownloadClientRecord) => Promise<void>;
  downloadClientOrder: string[];
  moveDownloadClient: (clientId: string, direction: "up" | "down") => Promise<void> | void;
  isSavingOrder: boolean;
};

const DOWNLOAD_CLIENT_TYPE_LOGO_OPTIONS: DownloadClientTypeLogoOption[] = [
  {
    value: "nzbget",
    iconSrc: "download-clients/nzbget.svg",
    icon: NzbgetIcon,
  },
  {
    value: "sabnzbd",
    iconSrc: "download-clients/sabnzbd.svg",
    icon: SabnzbdIcon,
  },
  {
    value: "weaver",
    iconSrc: "download-clients/weaver.svg",
    icon: WeaverIcon,
  },
  {
    value: "qbittorrent",
    iconSrc: "download-clients/qbittorrent.svg",
    icon: QBitTorrentIcon,
  },
];

function getDownloadClientTypeOption(typeValue: string) {
  const normalizedType = typeValue.trim().toLowerCase();
  return DOWNLOAD_CLIENT_TYPE_LOGO_OPTIONS.find((option) => option.value === normalizedType);
}

function DownloadClientTypeLogo({
  typeValue,
  className = "h-4 w-4",
}: {
  typeValue: string;
  className?: string;
}) {
  const option = getDownloadClientTypeOption(typeValue);
  const FallbackIcon = option?.icon ?? Server;
  const [failedToLoadImage, setFailedToLoadImage] = React.useState(false);

  if (failedToLoadImage || !option?.iconSrc) {
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

export function SettingsDownloadClientsSection({
  editingDownloadClientId,
  downloadClientTypeOptions,
  downloadClientDraft,
  setDownloadClientDraft,
  submitDownloadClient,
  testDownloadClientConnection,
  isTestingDownloadClientConnection,
  mutatingDownloadClientId,
  resetDownloadClientDraft,
  settingsDownloadClients,
  editDownloadClient,
  toggleDownloadClientEnabled,
  deleteDownloadClient,
  downloadClientOrder,
  moveDownloadClient,
  isSavingOrder,
}: SettingsDownloadClientsSectionProps) {
  const t = useTranslate();
  const urlPreview = buildUrlPreview(downloadClientDraft);
  const normalizedClientType = downloadClientDraft.clientType.trim().toLowerCase();
  const configuredClientLabel = downloadClientDraft.clientType.trim();
  const selectedDownloadClientLabel =
    downloadClientTypeOptions.find((option) => option.value === normalizedClientType)?.label ??
    (configuredClientLabel || "Download client");
  const hasApiKeyField =
    normalizedClientType === "sabnzbd" || normalizedClientType === "weaver";
  const weaverApiKeyUrl =
    normalizedClientType === "weaver" ? buildWeaverApiKeyUrl(downloadClientDraft) : "";

  const clientById = React.useMemo(
    () => Object.fromEntries(settingsDownloadClients.map((c) => [c.id, c])),
    [settingsDownloadClients],
  );
  const orderedClients = React.useMemo(() => {
    if (downloadClientOrder.length === 0) return settingsDownloadClients;
    const ordered: DownloadClientRecord[] = [];
    for (const id of downloadClientOrder) {
      const c = clientById[id];
      if (c) ordered.push(c);
    }
    for (const c of settingsDownloadClients) {
      if (!downloadClientOrder.includes(c.id)) ordered.push(c);
    }
    return ordered;
  }, [downloadClientOrder, clientById, settingsDownloadClients]);
  const hasOptionalCredentials = normalizedClientType === "nzbget";
  const optionalCredentialLabel = hasOptionalCredentials ? " (optional)" : "";

  return (
    <div className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <Server className="h-4 w-4" />
        {t("settings.downloadClientSection")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
                <TableRow>
                  <TableHead>{t("settings.downloadClientPriority")}</TableHead>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead className="text-center align-middle">
                    {t("label.type")}
                  </TableHead>
                  <TableHead>{t("settings.baseUrl")}</TableHead>
                  <TableHead className="text-center">{t("label.enabled")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
            </TableHeader>
            <TableBody>
              {orderedClients.map((client, index) => {
                return (
                  <TableRow key={client.id}>
                  <TableCell>
                    <div className="flex items-center gap-1">
                      <span className="w-4 text-center text-muted-foreground">{index + 1}</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        type="button"
                        className="border border-border bg-card/80 hover:bg-accent h-7 w-7 p-0"
                        aria-label={`${t("label.moveUp")} ${client.name}`}
                        onClick={() => moveDownloadClient(client.id, "up")}
                        disabled={isSavingOrder || index === 0}
                      >
                        <ChevronUp className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        type="button"
                        className="border border-border bg-card/80 hover:bg-accent h-7 w-7 p-0"
                        aria-label={`${t("label.moveDown")} ${client.name}`}
                        onClick={() => moveDownloadClient(client.id, "down")}
                        disabled={isSavingOrder || index >= orderedClients.length - 1}
                      >
                        <ChevronDown className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                  <TableCell>{client.name}</TableCell>
                  <TableCell className="text-center align-middle">
                    <span className="inline-flex items-center justify-center">
                      <DownloadClientTypeLogo typeValue={client.clientType} />
                      <span className="sr-only">{client.clientType}</span>
                    </span>
                  </TableCell>
                  <TableCell>{client.baseUrl || "—"}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={client.isEnabled}
                      label={`${t("label.enabled")}: ${client.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <DownloadClientActionButton
                        tone={client.isEnabled ? "disabled" : "enabled"}
                        onClick={() => void toggleDownloadClientEnabled(client)}
                        disabled={mutatingDownloadClientId === client.id}
                        label={client.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {client.isEnabled ? (
                          <PowerOff className="h-4 w-4" />
                        ) : (
                          <Power className="h-4 w-4" />
                        )}
                      </DownloadClientActionButton>
                      <DownloadClientActionButton
                        tone="edit"
                        onClick={() => editDownloadClient(client)}
                        label={t("label.edit")}
                      >
                        <Edit className="h-4 w-4" />
                      </DownloadClientActionButton>
                      <DownloadClientActionButton
                        tone="delete"
                        onClick={() => void deleteDownloadClient(client)}
                        disabled={mutatingDownloadClientId === client.id}
                        label={mutatingDownloadClientId === client.id ? t("label.deleting") : t("label.delete")}
                      >
                        <Trash2 className="h-4 w-4" />
                      </DownloadClientActionButton>
                    </div>
                  </TableCell>
                  </TableRow>
                );
              })}
              {orderedClients.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-muted-foreground">
                    {t("settings.noDownloadClientsFound")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingDownloadClientId ? t("settings.downloadClientUpdate") : t("settings.downloadClientCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-3" onSubmit={submitDownloadClient}>
            <div className="grid gap-3 md:grid-cols-3">
              <label>
                <Label className="mb-2 block">{t("label.name")}</Label>
                <Input
                  value={downloadClientDraft.name}
                  onChange={(event) =>
                    setDownloadClientDraft((prev: DownloadClientDraft) => ({
                      ...prev,
                      name: event.target.value,
                    }))
                  }
                  required
                  placeholder={t("settings.downloadClientNamePlaceholder")}
                />
              </label>
              <label>
                <Label className="mb-2 block">{t("label.type")}</Label>
                <Select
                  value={downloadClientDraft.clientType}
                  onValueChange={(value) =>
                    setDownloadClientDraft((prev: DownloadClientDraft) => ({
                      ...prev,
                      clientType: value,
                    }))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {downloadClientTypeOptions.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <div className="md:col-span-3 grid grid-cols-1 gap-2 md:grid-cols-[220px_92px_128px_auto] md:items-end md:gap-2">
                <label>
                  <Label className="mb-2 block">{t("settings.host")}</Label>
                  <Input
                    className="w-56 max-w-56"
                    value={downloadClientDraft.host}
                    onChange={(event) =>
                      setDownloadClientDraft((prev: DownloadClientDraft) => ({
                        ...prev,
                        host: event.target.value,
                      }))
                    }
                    required
                    placeholder={t("settings.downloadClientHostPlaceholder")}
                  />
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.port")}</Label>
                  <Input
                    {...integerInputProps}
                    value={downloadClientDraft.port}
                    onChange={(event) =>
                      setDownloadClientDraft((prev: DownloadClientDraft) => ({
                        ...prev,
                        port: sanitizeDigits(event.target.value),
                      }))
                    }
                    className="w-24 max-w-24"
                    placeholder={t("settings.downloadClientPortPlaceholder")}
                  />
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.downloadClientUrlBase")}</Label>
                  <Input
                    value={downloadClientDraft.urlBase}
                    onChange={(event) =>
                      setDownloadClientDraft((prev: DownloadClientDraft) => ({
                        ...prev,
                        urlBase: event.target.value,
                      }))
                    }
                    className="w-36 max-w-36"
                    placeholder={t("settings.downloadClientUrlBasePlaceholder")}
                  />
                </label>
                <label className="mb-2 ml-2 flex items-center gap-1.5 pl-2.5 md:ml-4">
                  <Checkbox
                    checked={downloadClientDraft.useSsl}
                    onCheckedChange={(checked) =>
                      setDownloadClientDraft((prev: DownloadClientDraft) => ({
                        ...prev,
                        useSsl: checked === true,
                      }))
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("settings.downloadClientUseSsl")}
                    <InfoHelp
                      ariaLabel={t("settings.downloadClientUseSsl")}
                      text={t("settings.downloadClientUseSslInfo")}
                    />
                  </span>
                </label>
              </div>
              <label className="md:col-span-3">
                <Label className="mb-2 block">{t("settings.downloadClientUrlPreview")}</Label>
                <Input value={urlPreview || "https://..."} readOnly disabled className="text-muted-foreground" />
              </label>
              {hasApiKeyField ? (
                <label>
                  <Label className="mb-2 block">{t("settings.apiKey")}</Label>
                  <Input
                    value={downloadClientDraft.apiKey}
                    onChange={(event) =>
                      setDownloadClientDraft((prev: DownloadClientDraft) => ({
                        ...prev,
                        apiKey: event.target.value,
                      }))
                    }
                    placeholder={t("form.apiKeyInputPlaceholder")}
                    type="password"
                  />
                  {normalizedClientType === "weaver" ? (
                    <p className="mt-2 text-xs text-muted-foreground">
                      Create an integration API key in Weaver:{" "}
                      {weaverApiKeyUrl ? (
                        <a
                          href={weaverApiKeyUrl}
                          target="_blank"
                          rel="noreferrer"
                          className="underline underline-offset-4 hover:text-foreground"
                        >
                          open Weaver security settings
                        </a>
                      ) : (
                        <span>finish the Weaver URL above to generate the link.</span>
                      )}
                    </p>
                  ) : null}
                </label>
              ) : null}
              {!hasApiKeyField ? (
                <>
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.username")}
                      {optionalCredentialLabel}
                    </Label>
                    <Input
                      value={downloadClientDraft.username}
                      onChange={(event) =>
                        setDownloadClientDraft((prev: DownloadClientDraft) => ({
                          ...prev,
                          username: event.target.value,
                        }))
                      }
                      placeholder={t("form.usernamePlaceholder")}
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.password")}
                      {optionalCredentialLabel}
                    </Label>
                    <Input
                      value={downloadClientDraft.password}
                      onChange={(event) =>
                        setDownloadClientDraft((prev: DownloadClientDraft) => ({
                          ...prev,
                          password: event.target.value,
                        }))
                      }
                      placeholder={t("form.passwordPlaceholder")}
                      type="password"
                    />
                  </label>
                </>
              ) : null}
              <details className="md:col-span-3 rounded-xl border border-border bg-card p-3" open>
                <summary className="cursor-pointer select-none text-sm font-medium text-card-foreground">
                  {t("qualityProfile.otherOptions")}
                </summary>
                <div className="mt-3 space-y-3">
                  <label className="mb-2 flex items-center gap-3">
                    <Checkbox
                      checked={downloadClientDraft.isEnabled}
                      onCheckedChange={(checked) =>
                        setDownloadClientDraft((prev: DownloadClientDraft) => ({
                          ...prev,
                          isEnabled: checked === true,
                        }))
                      }
                    />
                    <span className="inline-flex items-center gap-2 text-sm">
                      {t("form.enabled")}
                      <InfoHelp
                        ariaLabel={t("form.enabled")}
                        text={t("settings.downloadClientEnabledInfo")}
                      />
                    </span>
                  </label>
                </div>
              </details>
            </div>
            <div className="flex gap-2">
              <Button
                type="button"
                variant="secondary"
                onClick={() => void testDownloadClientConnection()}
                disabled={isTestingDownloadClientConnection || mutatingDownloadClientId !== null}
              >
                {isTestingDownloadClientConnection
                  ? t("status.testingDownloadClient", { client: selectedDownloadClientLabel })
                  : t("label.testConnection")}
              </Button>
              <Button type="submit" disabled={mutatingDownloadClientId === "new"}>
                {mutatingDownloadClientId === "new"
                  ? t("label.saving")
                  : editingDownloadClientId
                    ? t("settings.downloadClientUpdate")
                    : t("settings.downloadClientCreate")}
              </Button>
              <Button type="button" variant="secondary" onClick={resetDownloadClientDraft}>
                {t("label.cancel")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
