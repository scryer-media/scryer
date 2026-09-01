import { Button } from "@/components/ui/button";
import { TotpCodeForm } from "@/components/auth/totp-code-form";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { InfoHelp } from "@/components/common/info-help";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { AuthenticatedAvatar } from "@/components/common/authenticated-avatar";
import { Input, integerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Check, Loader2, Palette } from "lucide-react";
import { lazy, Suspense, useEffect, useState, type FormEvent } from "react";
import { useTheme } from "next-themes";
import { TotpQrCode } from "@/components/common/totp-qr-code";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type {
  ExternalAccountProvider,
  ExternalAuthRuntimeConnection,
  LinkedAccount,
  OAuthConnectedApp,
  PasskeySummary,
  TotpEnrollmentStart,
  TotpStatus,
  UiDateTimeFormat,
} from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import {
  applyHighlightColor,
  HIGHLIGHT_COLOR_PRESETS,
  isDarkTheme,
} from "@/lib/theme";
import { canSubmitJellyfinLink as canSubmitJellyfinLinkDraft } from "@/lib/utils/external-account-link-gate";

const TOTP_CODE_LENGTH = 6;

const HighlightColorPicker = lazy(() => import("./highlight-color-picker"));

type TotpProfileAction = "regenerateRecoveryCodes" | "disable" | null;

type Props = {
  username?: string;
  highlightColor: string | null;
  savingHighlightColor: string | null;
  onSelectHighlightColor: (value: string) => void;
  hideSponsorButton: boolean;
  savingSponsorPreference: boolean;
  onHideSponsorButtonChange: (value: boolean) => void;
  currentPassword: string;
  newPassword: string;
  confirmPassword: string;
  saving: boolean;
  canChangePassword: boolean;
  requiresCurrentPassword: boolean;
  onCurrentPasswordChange: (value: string) => void;
  onNewPasswordChange: (value: string) => void;
  onConfirmPasswordChange: (value: string) => void;
  onChangePassword: () => void;
  showPasskeys: boolean;
  canAddPasskey: boolean;
  passkeys: PasskeySummary[];
  oauthApps: OAuthConnectedApp[];
  totpStatus: TotpStatus | null;
  totpEnrollment: TotpEnrollmentStart | null;
  totpEnrollmentCode: string;
  totpActionCode: string;
  totpRecoveryCodes: string[];
  linkedAccounts: LinkedAccount[];
  linkedAccountConnectionLabels: Record<string, string>;
  linkableJellyfinConnections: ExternalAuthRuntimeConnection[];
  linkablePlexConnections: ExternalAuthRuntimeConnection[];
  linkableEmbyConnections: ExternalAuthRuntimeConnection[];
  linkingProvider: ExternalAccountProvider | null;
  linkAccountConnectionId: string;
  linkAccountUsername: string;
  linkAccountPassword: string;
  linkAccountEmbyMode: "LOCAL" | "CONNECT";
  linkAccountBusy: boolean;
  linkAccountError: string | null;
  loadingPasskeys: boolean;
  loadingOauthApps: boolean;
  loadingTotp: boolean;
  loadingLinkedAccounts: boolean;
  loadingLinkOptions: boolean;
  addingPasskey: boolean;
  totpBusy: boolean;
  deletingPasskeyId: string | null;
  revokingOauthGrantId: string | null;
  unlinkingAccountId: string | null;
  onAddPasskey: () => void;
  onDeletePasskey: (id: string) => Promise<void> | void;
  onRevokeOauthApp: (grantId: string) => void;
  onStartTotpEnrollment: () => void;
  onTotpEnrollmentCodeChange: (value: string) => void;
  onCompleteTotpEnrollment: () => void;
  onTotpActionCodeChange: (value: string) => void;
  onDisableTotp: () => void;
  onRegenerateTotpRecoveryCodes: () => void;
  onStartLinkAccount: (provider: ExternalAccountProvider) => void;
  onCancelLinkAccount: () => void;
  onLinkAccountConnectionChange: (value: string) => void;
  onLinkAccountUsernameChange: (value: string) => void;
  onLinkAccountPasswordChange: (value: string) => void;
  onLinkAccountEmbyModeChange: (value: "LOCAL" | "CONNECT") => void;
  onSubmitJellyfinLink: (event: FormEvent<HTMLFormElement>) => void;
  onSubmitEmbyLink: (event: FormEvent<HTMLFormElement>) => void;
  onSubmitPlexLink: (event: FormEvent<HTMLFormElement>) => void;
  onUnlinkExternalAccount: (id: string) => void;
};

function formatTimestamp(
  value: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
): string {
  return formatUiDateTime(value, dateTimeFormat, { fallback: "—" });
}

function providerLabel(provider: LinkedAccount["provider"]): string {
  switch (provider) {
    case "PLEX":
      return "Plex";
    case "JELLYFIN":
      return "Jellyfin";
    case "EMBY":
      return "Emby";
    default:
      return provider;
  }
}

function connectionLabel(connection: ExternalAuthRuntimeConnection): string {
  return connection.displayName;
}

function LinkedAccountAvatar({ account }: { account: LinkedAccount }) {
  const label = account.displayName || account.username;
  return (
    <AuthenticatedAvatar
      avatarUrl={account.avatarUrl}
      label={label}
      imageClassName="h-9 w-9 shrink-0 rounded-full border border-[var(--scry-border2)] object-cover"
      fallbackClassName="flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-sm font-medium text-[var(--scry-muted2)]"
    />
  );
}

const PROFILE_CARD_CLASS =
  "space-y-4 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const PROFILE_CARD_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const PROFILE_MUTED_TEXT_CLASS = "text-sm text-[var(--scry-muted3)]";
const PROFILE_ROW_CARD_CLASS =
  "flex flex-col gap-3 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 md:flex-row md:items-center md:justify-between";

export function SettingsProfileSection({
  username,
  highlightColor,
  savingHighlightColor,
  onSelectHighlightColor,
  hideSponsorButton,
  savingSponsorPreference,
  onHideSponsorButtonChange,
  currentPassword,
  newPassword,
  confirmPassword,
  saving,
  canChangePassword,
  requiresCurrentPassword,
  onCurrentPasswordChange,
  onNewPasswordChange,
  onConfirmPasswordChange,
  onChangePassword,
  showPasskeys,
  canAddPasskey,
  passkeys,
  oauthApps,
  totpStatus,
  totpEnrollment,
  totpEnrollmentCode,
  totpActionCode,
  totpRecoveryCodes,
  linkedAccounts,
  linkedAccountConnectionLabels,
  linkableJellyfinConnections,
  linkablePlexConnections,
  linkableEmbyConnections,
  linkingProvider,
  linkAccountConnectionId,
  linkAccountUsername,
  linkAccountPassword,
  linkAccountEmbyMode,
  linkAccountBusy,
  linkAccountError,
  loadingPasskeys,
  loadingOauthApps,
  loadingTotp,
  loadingLinkedAccounts,
  loadingLinkOptions,
  addingPasskey,
  totpBusy,
  deletingPasskeyId,
  revokingOauthGrantId,
  unlinkingAccountId,
  onAddPasskey,
  onDeletePasskey,
  onRevokeOauthApp,
  onStartTotpEnrollment,
  onTotpEnrollmentCodeChange,
  onCompleteTotpEnrollment,
  onTotpActionCodeChange,
  onDisableTotp,
  onRegenerateTotpRecoveryCodes,
  onStartLinkAccount,
  onCancelLinkAccount,
  onLinkAccountConnectionChange,
  onLinkAccountUsernameChange,
  onLinkAccountPasswordChange,
  onLinkAccountEmbyModeChange,
  onSubmitJellyfinLink,
  onSubmitEmbyLink,
  onSubmitPlexLink,
  onUnlinkExternalAccount,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { resolvedTheme, theme } = useTheme();
  const [pendingTotpAction, setPendingTotpAction] =
    useState<TotpProfileAction>(null);
  const [submittedTotpAction, setSubmittedTotpAction] = useState(false);
  const [pendingPasskeyDeletionId, setPendingPasskeyDeletionId] =
    useState<string | null>(null);
  const [customColorPickerOpen, setCustomColorPickerOpen] = useState(false);
  const [previewHighlightColor, setPreviewHighlightColor] = useState<
    string | null
  >(null);
  const darkThemeActive = isDarkTheme(resolvedTheme ?? theme);
  const savedHighlightColor =
    highlightColor ?? HIGHLIGHT_COLOR_PRESETS[0].value;
  const selectedHighlightColor =
    previewHighlightColor ?? savedHighlightColor;

  useEffect(() => {
    if (previewHighlightColor === null || typeof document === "undefined") {
      return undefined;
    }

    applyHighlightColor(
      document.documentElement,
      previewHighlightColor,
      darkThemeActive,
    );
    return () => {
      applyHighlightColor(
        document.documentElement,
        highlightColor,
        darkThemeActive,
      );
    };
  }, [darkThemeActive, highlightColor, previewHighlightColor]);

  const closeCustomColorPicker = () => {
    setPreviewHighlightColor(null);
    setCustomColorPickerOpen(false);
  };
  const passwordMismatch =
    confirmPassword.length > 0 && newPassword !== confirmPassword;
  const showSponsorButton = !hideSponsorButton;
  const canSubmit =
    (!requiresCurrentPassword || currentPassword.length > 0) &&
    newPassword.length > 0 &&
    !passwordMismatch &&
    !saving;
  const selectedLinkConnections =
    linkingProvider === "JELLYFIN"
      ? linkableJellyfinConnections
      : linkingProvider === "EMBY"
        ? linkableEmbyConnections
        : isVisibleExternalAccountProvider("PLEX")
        ? linkablePlexConnections
        : [];
  const selectedLinkConnection = selectedLinkConnections.find(
    (connection) => connection.id === linkAccountConnectionId,
  );
  const visibleLinkedAccounts = linkedAccounts.filter((account) =>
    isVisibleExternalAccountProvider(account.provider),
  );
  const canSubmitJellyfinLink = canSubmitJellyfinLinkDraft({
    connectionId: linkAccountConnectionId,
    username: linkAccountUsername,
    busy: linkAccountBusy,
  });
  const canSubmitPlexLink =
    Boolean(linkAccountConnectionId) && !linkAccountBusy;
  const closeTotpActionDialog = () => {
    setPendingTotpAction(null);
    setSubmittedTotpAction(false);
    onTotpActionCodeChange("");
  };
  const openTotpActionDialog = (action: Exclude<TotpProfileAction, null>) => {
    setPendingTotpAction(action);
    setSubmittedTotpAction(false);
    onTotpActionCodeChange("");
  };
  const handleSubmitTotpAction = () => {
    if (!pendingTotpAction || totpActionCode.length !== TOTP_CODE_LENGTH) {
      return;
    }

    setSubmittedTotpAction(true);
    if (pendingTotpAction === "regenerateRecoveryCodes") {
      onRegenerateTotpRecoveryCodes();
      return;
    }

    onDisableTotp();
  };

  useEffect(() => {
    if (
      pendingTotpAction &&
      submittedTotpAction &&
      !totpBusy &&
      totpActionCode.length === 0
    ) {
      setPendingTotpAction(null);
      setSubmittedTotpAction(false);
    }
  }, [pendingTotpAction, submittedTotpAction, totpActionCode.length, totpBusy]);

  return (
    <div
      id="settings-profile-section"
      className="space-y-4 text-sm text-[var(--scry-body)]"
    >
      <div className={PROFILE_CARD_CLASS}>
        <h3 className={PROFILE_CARD_TITLE_CLASS}>{t("profile.accountInfo")}</h3>
        <div className="flex items-center gap-2 text-[var(--scry-muted3)]">
          <span>{t("settings.username")}:</span>
          <span
            id="settings-profile-username"
            className="font-medium text-[var(--scry-ink2)]"
          >
            {username ?? "—"}
          </span>
        </div>
      </div>

      <div className={PROFILE_CARD_CLASS}>
        <div className="space-y-1">
          <h3 className={PROFILE_CARD_TITLE_CLASS}>{t("profile.appearance")}</h3>
          <p className={PROFILE_MUTED_TEXT_CLASS}>
            {t("profile.appearanceDescription")}
          </p>
        </div>
        <div className={PROFILE_ROW_CARD_CLASS}>
          <div className="space-y-1">
            <div className="font-medium text-[var(--scry-ink2)]">
              {t("profile.highlightColor")}
            </div>
            <p className={`${PROFILE_MUTED_TEXT_CLASS} max-w-xs`}>
              {t("profile.highlightColorHelp")}
            </p>
          </div>
          <div
            id="settings-profile-highlight-colors"
            role="group"
            aria-label={t("profile.highlightColor")}
            className="flex flex-wrap items-center gap-2.5"
          >
            {HIGHLIGHT_COLOR_PRESETS.map((preset) => {
              const selected = selectedHighlightColor === preset.value;
              const savingThis = savingHighlightColor === preset.value;
              return (
                <button
                  key={preset.value}
                  id={selectorId(
                    "settings-profile-highlight-color",
                    preset.value.replace("#", ""),
                  )}
                  type="button"
                  title={t(preset.labelKey)}
                  aria-label={t(preset.labelKey)}
                  aria-pressed={selected}
                  disabled={savingHighlightColor !== null}
                  onClick={() => onSelectHighlightColor(preset.value)}
                  className={cn(
                    "relative size-8 shrink-0 rounded-[9px] border-0 outline-none transition",
                    "focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--scry-card2)]",
                    "disabled:cursor-not-allowed",
                    selected
                      ? "scale-105"
                      : "enabled:hover:scale-105 disabled:opacity-60",
                  )}
                  style={{ backgroundColor: preset.value }}
                >
                  {savingThis ? (
                    <Loader2 className="absolute inset-0 m-auto h-3.5 w-3.5 animate-spin text-white" />
                  ) : selected ? (
                    <Check className="absolute inset-0 m-auto h-4 w-4 text-white drop-shadow-[0_1px_2px_rgba(0,0,0,0.8)]" />
                  ) : null}
                </button>
              );
            })}
            <Popover
              open={customColorPickerOpen}
              onOpenChange={(open) => {
                if (open) {
                  setCustomColorPickerOpen(true);
                } else {
                  closeCustomColorPicker();
                }
              }}
            >
              <PopoverTrigger asChild>
                <Button
                  id="settings-profile-highlight-color-custom"
                  type="button"
                  variant="outline"
                  size="icon-sm"
                  title={`${t("profile.highlightColorCustom")}: ${selectedHighlightColor}`}
                  aria-label={`${t("profile.highlightColorCustom")}: ${selectedHighlightColor}`}
                  disabled={savingHighlightColor !== null}
                  className="rounded-[9px] border-0 p-0 text-white shadow-none hover:brightness-110 hover:text-white"
                  style={{ backgroundColor: selectedHighlightColor }}
                >
                  <Palette className="size-5 drop-shadow-[0_1px_2px_rgba(0,0,0,0.9)]" />
                </Button>
              </PopoverTrigger>
              <PopoverContent
                align="end"
                sideOffset={8}
                className="w-[min(22rem,calc(100vw-2rem))] border-primary/45 bg-[var(--scry-card2)] p-4"
              >
                {customColorPickerOpen ? (
                  <Suspense
                    fallback={
                      <div className="flex h-64 items-center justify-center">
                        <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-muted3)]" />
                      </div>
                    }
                  >
                    <HighlightColorPicker
                      value={selectedHighlightColor}
                      onPreview={setPreviewHighlightColor}
                      onApply={(value) => {
                        setPreviewHighlightColor(null);
                        onSelectHighlightColor(value);
                        setCustomColorPickerOpen(false);
                      }}
                      onCancel={closeCustomColorPicker}
                    />
                  </Suspense>
                ) : null}
              </PopoverContent>
            </Popover>
          </div>
        </div>
        <div className={PROFILE_ROW_CARD_CLASS}>
          <div className="space-y-1">
            <div className="font-medium text-[var(--scry-ink2)]">
              Sponsor button
            </div>
            <p className={`${PROFILE_MUTED_TEXT_CLASS} max-w-xs`}>
              Show the Sponsor link in the navigation footer.
            </p>
          </div>
          <div className="flex items-center gap-2">
            {savingSponsorPreference ? (
              <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-muted3)]" />
            ) : null}
            <SettingsToggleSwitch
              id="settings-profile-hide-sponsor-button"
              checked={showSponsorButton}
              disabled={savingSponsorPreference}
              onChange={(visible) => onHideSponsorButtonChange(!visible)}
              ariaLabel="Show Sponsor button"
            />
          </div>
        </div>
      </div>

      {canChangePassword ? (
        <div className={PROFILE_CARD_CLASS}>
          <div className="space-y-4">
            <h3 className={PROFILE_CARD_TITLE_CLASS}>
              {t("profile.changePassword")}
            </h3>
            <div className="grid max-w-sm gap-3">
              {requiresCurrentPassword ? (
                <div className="space-y-1.5">
                  <Label htmlFor="current-password">
                    {t("profile.currentPassword")}
                  </Label>
                  <Input
                    id="current-password"
                    type="password"
                    autoComplete="current-password"
                    value={currentPassword}
                    onChange={(e) => onCurrentPasswordChange(e.target.value)}
                  />
                </div>
              ) : null}
              <div className="space-y-1.5">
                <Label htmlFor="new-password">{t("profile.newPassword")}</Label>
                <Input
                  id="new-password"
                  type="password"
                  autoComplete="new-password"
                  value={newPassword}
                  onChange={(e) => onNewPasswordChange(e.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="confirm-password">
                  {t("profile.confirmPassword")}
                </Label>
                <Input
                  id="confirm-password"
                  type="password"
                  autoComplete="new-password"
                  value={confirmPassword}
                  onChange={(e) => onConfirmPasswordChange(e.target.value)}
                />
                {passwordMismatch ? (
                  <p className="text-xs text-destructive">
                    {t("profile.passwordMismatch")}
                  </p>
                ) : null}
              </div>
              <Button
                id={selectorId("settings-profile-change-password")}
                onClick={onChangePassword}
                disabled={!canSubmit}
                className="w-fit"
              >
                {saving ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.changePassword")}
              </Button>
            </div>
          </div>
        </div>
      ) : null}

      <div className={cn("grid gap-4", showPasskeys && "xl:grid-cols-2")}>
        {showPasskeys ? (
          <div className={PROFILE_CARD_CLASS}>
          <div className="space-y-4">
            <div className="flex items-center gap-1.5">
              <h3 className={PROFILE_CARD_TITLE_CLASS}>
                {t("profile.passkeys")}
              </h3>
              <InfoHelp
                ariaLabel={t("profile.passkeys")}
                text={t("profile.passkeysDescription")}
              />
            </div>

            {canAddPasskey ? (
              <Button
                id={selectorId("settings-profile-add-passkey")}
                onClick={onAddPasskey}
                disabled={addingPasskey}
                className="w-fit"
              >
                {addingPasskey ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {addingPasskey
                  ? t("profile.passkeyAdding")
                  : t("profile.passkeyAdd")}
              </Button>
            ) : null}

            {loadingPasskeys ? (
              <div className="flex items-center gap-2 text-sm text-[var(--scry-muted3)]">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("label.loading")}</span>
              </div>
            ) : passkeys.length > 0 ? (
              <div className="space-y-3">
                {passkeys.map((passkey) => (
                  <div key={passkey.id} className={PROFILE_ROW_CARD_CLASS}>
                    <div className="space-y-1">
                      <div className="font-medium text-[var(--scry-ink2)]">
                        {passkey.friendlyName || t("profile.passkeyLabel")}
                      </div>
                      <div className={PROFILE_MUTED_TEXT_CLASS}>
                        {t("profile.passkeyCreatedAt")}:{" "}
                        {formatTimestamp(passkey.createdAt, dateTimeFormat)}
                      </div>
                      <div className={PROFILE_MUTED_TEXT_CLASS}>
                        {t("profile.passkeyLastUsedAt")}:{" "}
                        {passkey.lastUsedAt
                          ? formatTimestamp(passkey.lastUsedAt, dateTimeFormat)
                          : t("profile.passkeyNeverUsed")}
                      </div>
                    </div>
                    <Button
                      id={selectorId(
                        `settings-profile-delete-passkey-${passkey.id}`,
                      )}
                      variant="destructive"
                      onClick={() => setPendingPasskeyDeletionId(passkey.id)}
                      disabled={deletingPasskeyId === passkey.id}
                      className="w-fit"
                    >
                      {deletingPasskeyId === passkey.id ? (
                        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      ) : null}
                      {t("label.delete")}
                    </Button>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        </div>
        ) : null}

        <div className={PROFILE_CARD_CLASS}>
        <div className="flex items-center gap-1.5">
          <h3 className={PROFILE_CARD_TITLE_CLASS}>{t("profile.totp")}</h3>
          <InfoHelp
            ariaLabel={t("profile.totp")}
            text={t("profile.totpDescription")}
          />
        </div>

        {loadingTotp ? (
          <div className="flex items-center gap-2 text-sm text-[var(--scry-muted3)]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : totpStatus?.enabled ? (
          <div className="space-y-3 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4">
            <div className="grid gap-2 text-sm text-[var(--scry-muted3)] sm:grid-cols-3">
              <div>
                <span className="text-[var(--scry-ink2)]">
                  {t("profile.totpEnabledAt")}:{" "}
                </span>
                {formatTimestamp(totpStatus.createdAt, dateTimeFormat)}
              </div>
              <div>
                <span className="text-[var(--scry-ink2)]">
                  {t("profile.totpLastUsedAt")}:{" "}
                </span>
                {formatTimestamp(totpStatus.lastUsedAt, dateTimeFormat)}
              </div>
              <div>
                <span className="text-[var(--scry-ink2)]">
                  {t("profile.totpRecoveryRemaining")}:{" "}
                </span>
                {totpStatus.recoveryCodesRemaining}
              </div>
            </div>

            {totpRecoveryCodes.length > 0 ? (
              <div className="rounded-md border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3">
                <div className="mb-2 text-sm font-medium">
                  {t("profile.totpRecoveryCodes")}
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  {totpRecoveryCodes.map((code) => (
                    <code
                      id={selectorId(
                        "settings-profile-totp-recovery-code",
                        code,
                      )}
                      key={code}
                      className="rounded bg-background/70 px-2 py-1 font-[var(--font-code)] text-xs"
                    >
                      {code}
                    </code>
                  ))}
                </div>
              </div>
            ) : null}

            <div className="flex flex-wrap gap-2">
              <Button
                id={selectorId(
                  "settings-profile-totp-regenerate-recovery-codes",
                )}
                type="button"
                variant="outline"
                disabled={totpBusy}
                onClick={() => openTotpActionDialog("regenerateRecoveryCodes")}
              >
                {totpBusy ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.totpRegenerateRecoveryCodes")}
              </Button>
              <Button
                id={selectorId("settings-profile-totp-disable")}
                type="button"
                variant="destructive"
                disabled={totpBusy}
                onClick={() => openTotpActionDialog("disable")}
              >
                {totpBusy ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.totpDisable")}
              </Button>
            </div>

            <Dialog
              open={pendingTotpAction !== null}
              onOpenChange={(open) => {
                if (!open) {
                  closeTotpActionDialog();
                }
              }}
            >
              <DialogContent
                id="settings-profile-totp-action-dialog"
                className="sm:max-w-md"
                onInteractOutside={(event) => event.preventDefault()}
              >
                <DialogHeader>
                  <DialogTitle>
                    {pendingTotpAction === "regenerateRecoveryCodes"
                      ? t("profile.totpRegenerateRecoveryCodes")
                      : t("profile.totpDisable")}
                  </DialogTitle>
                  <DialogDescription>
                    {t("profile.totpActionDescription")}
                  </DialogDescription>
                </DialogHeader>
                <TotpCodeForm
                  inputId="totp-action-code"
                  submitId={
                    pendingTotpAction === "regenerateRecoveryCodes"
                      ? selectorId(
                          "settings-profile-totp-regenerate-recovery-codes-confirm",
                        )
                      : selectorId("settings-profile-totp-disable-confirm")
                  }
                  cancelId={selectorId("settings-profile-totp-action-cancel")}
                  code={totpActionCode}
                  title={t("profile.totpCode")}
                  description={t("profile.totpActionDescription")}
                  submitLabel={
                    pendingTotpAction === "regenerateRecoveryCodes"
                      ? t("profile.totpRegenerateRecoveryCodes")
                      : t("profile.totpDisable")
                  }
                  cancelLabel={t("label.cancel")}
                  busy={totpBusy}
                  onCodeChange={onTotpActionCodeChange}
                  onSubmit={handleSubmitTotpAction}
                  onCancel={closeTotpActionDialog}
                />
              </DialogContent>
            </Dialog>
          </div>
        ) : totpEnrollment ? (
          <div className="space-y-4 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4">
            <div className="flex flex-col gap-4 sm:flex-row">
              <TotpQrCode
                id={selectorId("settings-profile-totp-qr-code")}
                value={totpEnrollment.otpauthUrl}
              />
              <div className="min-w-0 space-y-3">
                <a
                  id={selectorId("settings-profile-totp-setup-link")}
                  className="break-all text-sm font-medium text-primary underline-offset-4 hover:underline"
                  href={totpEnrollment.otpauthUrl}
                >
                  {t("profile.totpOpenSetupLink")}
                </a>
                <div className="space-y-1">
                  <div className="text-xs text-muted-foreground">
                    {t("profile.totpSecret")}
                  </div>
                  <code
                    id={selectorId("settings-profile-totp-secret")}
                    className="block break-all rounded bg-background/70 px-2 py-1 font-[var(--font-code)] text-xs"
                  >
                    {totpEnrollment.secretBase32}
                  </code>
                </div>
              </div>
            </div>
            <div className="grid max-w-sm gap-2">
              <Label htmlFor="totp-enrollment-code">
                {t("profile.totpCode")}
              </Label>
              <Input
                id="totp-enrollment-code"
                {...integerInputProps}
                inputMode="numeric"
                maxLength={TOTP_CODE_LENGTH}
                autoComplete="one-time-code"
                value={totpEnrollmentCode}
                onChange={(event) =>
                  onTotpEnrollmentCodeChange(event.target.value)
                }
              />
              <Button
                id={selectorId("settings-profile-totp-verify-enable")}
                type="button"
                disabled={
                  totpBusy || totpEnrollmentCode.length !== TOTP_CODE_LENGTH
                }
                onClick={onCompleteTotpEnrollment}
                className="w-fit"
              >
                {totpBusy ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.totpVerifyAndEnable")}
              </Button>
            </div>
          </div>
        ) : (
          <Button
            id={selectorId("settings-profile-start-totp")}
            type="button"
            onClick={onStartTotpEnrollment}
            disabled={totpBusy}
            className="w-fit"
          >
            {totpBusy ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("profile.totpStartEnrollment")}
          </Button>
        )}
        </div>
      </div>

      <div className={PROFILE_CARD_CLASS}>
        <div className="space-y-1">
          <h3 className={PROFILE_CARD_TITLE_CLASS}>Connected apps</h3>
          <p className={PROFILE_MUTED_TEXT_CLASS}>
            OAuth integrations authorized to access Scryer as you.
          </p>
        </div>

        {loadingOauthApps ? (
          <div className="flex items-center gap-2 text-sm text-[var(--scry-muted3)]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : oauthApps.length === 0 ? (
          <p
            id="settings-profile-oauth-apps-empty"
            className={PROFILE_MUTED_TEXT_CLASS}
          >
            No connected apps.
          </p>
        ) : (
          <div className="space-y-3">
            {oauthApps.map((app) => (
              <div
                id={selectorId("settings-profile-oauth-app-row", app.clientId)}
                key={app.grantId}
                className={PROFILE_ROW_CARD_CLASS}
              >
                <div className="space-y-1">
                  <div className="font-medium text-[var(--scry-ink2)]">
                    {app.clientName}
                  </div>
                  <div
                    id={selectorId(
                      "settings-profile-oauth-app-authorized-at",
                      app.clientId,
                    )}
                    className={PROFILE_MUTED_TEXT_CLASS}
                  >
                    Authorized: {formatTimestamp(app.authorizedAt, dateTimeFormat)}
                  </div>
                  <div
                    id={selectorId(
                      "settings-profile-oauth-app-last-used",
                      app.clientId,
                    )}
                    className={PROFILE_MUTED_TEXT_CLASS}
                  >
                    Last used: {formatTimestamp(app.lastUsedAt, dateTimeFormat)}
                  </div>
                </div>
                <Button
                  id={selectorId(
                    "settings-profile-revoke-oauth-app",
                    app.clientId,
                  )}
                  variant="outline"
                  onClick={() => onRevokeOauthApp(app.grantId)}
                  disabled={revokingOauthGrantId === app.grantId}
                  className="w-fit"
                >
                  {revokingOauthGrantId === app.grantId ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : null}
                  Revoke
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className={PROFILE_CARD_CLASS}>
        <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
          <div className="space-y-1">
            <h3 className={PROFILE_CARD_TITLE_CLASS}>
              {t("profile.linkedAccounts")}
            </h3>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {loadingLinkOptions ? (
              <div className="flex items-center gap-2 text-sm text-[var(--scry-muted3)]">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("label.loading")}</span>
              </div>
            ) : null}
            {linkableJellyfinConnections.length > 0 ? (
              <Button
                id="settings-profile-link-jellyfin-start"
                type="button"
                variant={
                  linkingProvider === "JELLYFIN" ? "secondary" : "outline"
                }
                onClick={() => onStartLinkAccount("JELLYFIN")}
                disabled={linkAccountBusy}
              >
                {t("profile.linkJellyfinAccount")}
              </Button>
            ) : null}
            {linkableEmbyConnections.length > 0 ? (
              <Button
                id="profile-link-emby"
                type="button"
                variant={linkingProvider === "EMBY" ? "secondary" : "outline"}
                onClick={() => onStartLinkAccount("EMBY")}
                disabled={linkAccountBusy}
              >
                Link Emby account
              </Button>
            ) : null}
            {isVisibleExternalAccountProvider("PLEX") &&
            linkablePlexConnections.length > 0 ? (
              <Button
                id="settings-profile-link-plex-start"
                type="button"
                variant={linkingProvider === "PLEX" ? "secondary" : "outline"}
                onClick={() => onStartLinkAccount("PLEX")}
                disabled={linkAccountBusy}
              >
                {t("profile.linkPlexAccount")}
              </Button>
            ) : null}
          </div>
        </div>

        {linkingProvider === "JELLYFIN" ? (
          <form
            className="grid gap-3 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 md:max-w-xl"
            onSubmit={onSubmitJellyfinLink}
          >
            <div className="space-y-1.5">
              <Label htmlFor="profile-link-jellyfin-connection">
                {t("profile.linkAccountConnection")}
              </Label>
              {selectedLinkConnections.length > 1 ? (
                <Select
                  value={linkAccountConnectionId}
                  onValueChange={onLinkAccountConnectionChange}
                  disabled={linkAccountBusy}
                >
                  <SelectTrigger
                    id="profile-link-jellyfin-connection"
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {selectedLinkConnections.map((connection) => (
                      <SelectItem
                        id={selectorId(
                          "profile-link-jellyfin-connection-option",
                          connection.id,
                        )}
                        key={connection.id}
                        value={connection.id}
                      >
                        {connectionLabel(connection)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <div className="rounded-md border border-input bg-muted/40 px-3 py-2 text-sm">
                  {selectedLinkConnection
                    ? connectionLabel(selectedLinkConnection)
                    : t("profile.linkAccountNoConnections")}
                </div>
              )}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label htmlFor="profile-link-jellyfin-username">
                  {t("profile.linkAccountUsername")}
                </Label>
                <Input
                  id="profile-link-jellyfin-username"
                  type="text"
                  autoComplete="username"
                  value={linkAccountUsername}
                  onChange={(event) =>
                    onLinkAccountUsernameChange(event.target.value)
                  }
                  disabled={linkAccountBusy}
                />
              </div>
              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5">
                  <Label htmlFor="profile-link-jellyfin-password">
                    {t("profile.linkAccountPassword")}
                  </Label>
                  <InfoHelp
                    ariaLabel={t("profile.linkAccountPassword")}
                    text={t("profile.linkAccountPasswordlessHint")}
                  />
                </div>
                <Input
                  id="profile-link-jellyfin-password"
                  type="password"
                  autoComplete="current-password"
                  value={linkAccountPassword}
                  onChange={(event) =>
                    onLinkAccountPasswordChange(event.target.value)
                  }
                  disabled={linkAccountBusy}
                />
              </div>
            </div>
            {linkAccountError ? (
              <p
                id="settings-profile-link-jellyfin-error"
                className="text-sm text-destructive"
              >
                {linkAccountError}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button
                id="settings-profile-link-jellyfin-submit"
                type="submit"
                disabled={!canSubmitJellyfinLink}
                className="w-fit"
              >
                {linkAccountBusy ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.linkAccountSubmit")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={onCancelLinkAccount}
                disabled={linkAccountBusy}
                className="w-fit"
              >
                {t("profile.linkAccountCancel")}
              </Button>
            </div>
          </form>
        ) : null}

        {linkingProvider === "EMBY" ? (
          <form
            className="grid gap-3 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 md:max-w-xl"
            onSubmit={onSubmitEmbyLink}
          >
            <div className="space-y-1.5">
              <Label htmlFor="profile-link-emby-connection">
                {t("profile.linkAccountConnection")}
              </Label>
              <Select
                value={linkAccountConnectionId}
                onValueChange={onLinkAccountConnectionChange}
                disabled={linkAccountBusy}
              >
                <SelectTrigger id="profile-link-emby-connection" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {selectedLinkConnections.map((connection) => (
                    <SelectItem
                      id={selectorId("profile-link-emby-connection-option", connection.id)}
                      key={connection.id}
                      value={connection.id}
                    >
                      {connectionLabel(connection)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {selectedLinkConnection?.embyConnectEnabled ? (
              <div className="grid grid-cols-2 gap-2">
                <Button
                  id="profile-link-emby-mode-local"
                  type="button"
                  variant={linkAccountEmbyMode === "LOCAL" ? "secondary" : "outline"}
                  onClick={() => onLinkAccountEmbyModeChange("LOCAL")}
                >
                  Local
                </Button>
                <Button
                  id="profile-link-emby-mode-connect"
                  type="button"
                  variant={linkAccountEmbyMode === "CONNECT" ? "secondary" : "outline"}
                  onClick={() => onLinkAccountEmbyModeChange("CONNECT")}
                >
                  Connect
                </Button>
              </div>
            ) : null}
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label htmlFor="profile-link-emby-username">
                  {linkAccountEmbyMode === "CONNECT"
                    ? "Emby Connect username or email"
                    : t("profile.linkAccountUsername")}
                </Label>
                <Input
                  id="profile-link-emby-username"
                  value={linkAccountUsername}
                  onChange={(event) => onLinkAccountUsernameChange(event.target.value)}
                  disabled={linkAccountBusy}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="profile-link-emby-password">
                  {t("profile.linkAccountPassword")}
                </Label>
                <Input
                  id="profile-link-emby-password"
                  type="password"
                  value={linkAccountPassword}
                  onChange={(event) => onLinkAccountPasswordChange(event.target.value)}
                  disabled={linkAccountBusy}
                />
              </div>
            </div>
            {linkAccountError ? (
              <p id="profile-link-emby-error" className="text-sm text-destructive">
                {linkAccountError}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button
                id="profile-link-emby-submit"
                type="submit"
                disabled={linkAccountBusy || !linkAccountConnectionId || !linkAccountUsername.trim()}
              >
                {linkAccountBusy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                {t("profile.linkAccountSubmit")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={onCancelLinkAccount}
                disabled={linkAccountBusy}
              >
                {t("profile.linkAccountCancel")}
              </Button>
            </div>
          </form>
        ) : null}

        {isVisibleExternalAccountProvider("PLEX") &&
        linkingProvider === "PLEX" ? (
          <form
            className="grid gap-3 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 md:max-w-xl"
            onSubmit={onSubmitPlexLink}
          >
            <div className="space-y-1.5">
              <Label htmlFor="profile-link-plex-connection">
                {t("profile.linkAccountConnection")}
              </Label>
              {selectedLinkConnections.length > 1 ? (
                <Select
                  value={linkAccountConnectionId}
                  onValueChange={onLinkAccountConnectionChange}
                  disabled={linkAccountBusy}
                >
                  <SelectTrigger
                    id="profile-link-plex-connection"
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {selectedLinkConnections.map((connection) => (
                      <SelectItem
                        id={selectorId(
                          "profile-link-plex-connection-option",
                          connection.id,
                        )}
                        key={connection.id}
                        value={connection.id}
                      >
                        {connectionLabel(connection)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <div className="rounded-md border border-input bg-muted/40 px-3 py-2 text-sm">
                  {selectedLinkConnection
                    ? connectionLabel(selectedLinkConnection)
                    : t("profile.linkAccountNoConnections")}
                </div>
              )}
            </div>
            {linkAccountError ? (
              <p
                id="settings-profile-link-plex-error"
                className="text-sm text-destructive"
              >
                {linkAccountError}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button
                id="settings-profile-link-plex-submit"
                type="submit"
                disabled={!canSubmitPlexLink}
                className="w-fit"
              >
                {linkAccountBusy ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : null}
                {t("profile.signInWithPlexToLink")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={onCancelLinkAccount}
                disabled={linkAccountBusy}
                className="w-fit"
              >
                {t("profile.linkAccountCancel")}
              </Button>
            </div>
          </form>
        ) : null}

        {loadingLinkedAccounts ? (
          <div className="flex items-center gap-2 text-sm text-[var(--scry-muted3)]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : visibleLinkedAccounts.length === 0 ? (
          <p className={PROFILE_MUTED_TEXT_CLASS}>
            {t("profile.linkedAccountsEmpty")}
          </p>
        ) : (
          <div className="space-y-3">
            {visibleLinkedAccounts.map((account) => (
              <div
                id={selectorId(
                  "settings-profile-linked-account",
                  account.provider,
                  account.username,
                )}
                key={account.id}
                className={PROFILE_ROW_CARD_CLASS}
              >
                <div className="flex min-w-0 items-start gap-3">
                  <LinkedAccountAvatar account={account} />
                  <div className="min-w-0 space-y-1">
                    <div className="truncate font-medium text-[var(--scry-ink2)]">
                      {providerLabel(account.provider)} ·{" "}
                      {account.displayName || account.username}
                    </div>
                    <div className={PROFILE_MUTED_TEXT_CLASS}>
                      {t("profile.linkedAccountConnection")}:{" "}
                      {linkedAccountConnectionLabels[
                        `${account.provider}:${account.connectionId}`
                      ] ?? t("profile.linkedAccountUnknownConnection")}
                    </div>
                    <div className={PROFILE_MUTED_TEXT_CLASS}>
                      {t("profile.linkedAccountStatus")}: {account.status}
                    </div>
                  </div>
                </div>
                <Button
                  id={selectorId(
                    "settings-profile-unlink-account",
                    account.provider,
                    account.username,
                  )}
                  variant="outline"
                  onClick={() => onUnlinkExternalAccount(account.id)}
                  disabled={unlinkingAccountId === account.id}
                  className="w-fit"
                >
                  {unlinkingAccountId === account.id ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : null}
                  {t("profile.unlinkAccount")}
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>
      <ConfirmDialog
        open={pendingPasskeyDeletionId !== null}
        title={t("profile.passkeyDeleteConfirmTitle")}
        description={t("profile.passkeyDeleteConfirmDescription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-profile-delete-passkey-confirm"
        confirmButtonVariant="destructive"
        isBusy={deletingPasskeyId === pendingPasskeyDeletionId}
        onConfirm={async () => {
          if (!pendingPasskeyDeletionId) return;
          await onDeletePasskey(pendingPasskeyDeletionId);
          setPendingPasskeyDeletionId(null);
        }}
        onCancel={() => setPendingPasskeyDeletionId(null)}
      />
    </div>
  );
}
