import type * as React from "react";
import { InfoHelp } from "@/components/common/info-help";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { CheckboxField } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { SecuritySettings } from "@/lib/types/settings";

const SECURITY_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const SECURITY_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const SECURITY_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const SECURITY_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";

type SettingsSecuritySectionProps = {
  settings: SecuritySettings;
  loading: boolean;
  enableConfirmOpen: boolean;
  disableConfirmOpen: boolean;
  setPasswordOpen: boolean;
  newPassword: string;
  newPasswordConfirm: string;
  setPasswordError: string | null;
  confirmBusy: boolean;
  confirmPassword: string;
  confirmError: string | null;
  passwordMinLengthDraft: string;
  minPasswordLength: number;
  onToggle: (enabled: boolean) => void;
  onConfirmPasswordChange: (value: string) => void;
  onConfirmEnable: () => Promise<void> | void;
  onCancelEnable: () => void;
  onConfirmDisable: () => Promise<void> | void;
  onCancelDisable: () => void;
  onNewPasswordChange: (value: string) => void;
  onNewPasswordConfirmChange: (value: string) => void;
  onConfirmSetPassword: () => Promise<void> | void;
  onCancelSetPassword: () => void;
  onPasswordMinLengthDraftChange: (value: string) => void;
  onPasswordMinLengthSubmit: (value?: string) => Promise<void> | void;
  onSkipLocalIpsChange: (enabled: boolean) => void;
  onApiKeysRestrictionChange: (enabled: boolean) => void;
  canManageApiKeysRestriction: boolean;
  onMfaConfigStepUpChange: (enabled: boolean) => void;
  onMfaPasswordLoginChange: (enabled: boolean) => void;
  onTotpJellyfinLoginChange: (enabled: boolean) => void;
  onTotpEmbyLoginChange: (enabled: boolean) => void;
  externalAccountInvitesPanel: React.ReactNode;
  oauthApplicationsPanel: React.ReactNode;
};

export function SettingsSecuritySection({
  settings,
  loading,
  enableConfirmOpen,
  disableConfirmOpen,
  setPasswordOpen,
  newPassword,
  newPasswordConfirm,
  setPasswordError,
  confirmBusy,
  confirmPassword,
  confirmError,
  passwordMinLengthDraft,
  minPasswordLength,
  onToggle,
  onConfirmPasswordChange,
  onConfirmEnable,
  onCancelEnable,
  onConfirmDisable,
  onCancelDisable,
  onNewPasswordChange,
  onNewPasswordConfirmChange,
  onConfirmSetPassword,
  onCancelSetPassword,
  onPasswordMinLengthDraftChange,
  onPasswordMinLengthSubmit,
  onSkipLocalIpsChange,
  onApiKeysRestrictionChange,
  canManageApiKeysRestriction,
  onMfaConfigStepUpChange,
  onMfaPasswordLoginChange,
  onTotpJellyfinLoginChange,
  onTotpEmbyLoginChange,
  externalAccountInvitesPanel,
  oauthApplicationsPanel,
}: SettingsSecuritySectionProps) {
  const t = useTranslate();
  const busy = loading || confirmBusy;
  const confirmDisabled = confirmPassword.trim().length === 0;
  const setPasswordDisabled =
    newPassword.length === 0 || newPasswordConfirm.length === 0;

  return (
    <>
      <div id="settings-security-section" className="space-y-4 text-sm">
        <div className={SECURITY_PANEL_CLASS}>
          <div className={SECURITY_PANEL_HEADER_CLASS}>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="space-y-1">
                <h3 className={SECURITY_PANEL_TITLE_CLASS}>
                  {t("settings.securityEnableFormLogin")}
                </h3>
                <p className="text-xs text-[var(--scry-muted3)]">
                  {t("settings.securityEnableFormLoginHelp")}
                </p>
              </div>
              <Button
                id="settings-security-toggle-form-login"
                type="button"
                aria-pressed={settings.formLoginEnabled}
                variant={settings.formLoginEnabled ? "destructive" : "primary"}
                disabled={busy}
                className="shrink-0 self-start sm:self-auto"
                onClick={() => onToggle(!settings.formLoginEnabled)}
              >
                {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {settings.formLoginEnabled ? t("label.disable") : t("label.enable")}
              </Button>
            </div>
          </div>
          <div className="grid gap-4 p-4 lg:grid-cols-2">
            <section className={`${SECURITY_INSET_CLASS} p-4`}>
              <div className="mb-3">
                <h4 className="text-sm font-semibold text-[var(--scry-ink2)]">
                  Password policy
                </h4>
              </div>
              <div className="max-w-sm space-y-1.5">
                <Label className="text-sm font-medium" htmlFor="security-password-min-length">
                  {t("settings.securityPasswordMinLength")}
                </Label>
                <Input
                  id="security-password-min-length"
                  type="number"
                  inputMode="numeric"
                  min={minPasswordLength}
                  step={1}
                  value={passwordMinLengthDraft}
                  disabled={busy}
                  onBlur={(event) =>
                    void onPasswordMinLengthSubmit(event.currentTarget.value)
                  }
                  onChange={(event) => onPasswordMinLengthDraftChange(event.target.value)}
                  onWheel={(event) => event.currentTarget.blur()}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void onPasswordMinLengthSubmit(event.currentTarget.value);
                    }
                  }}
                />
                <p className="text-xs leading-relaxed text-[var(--scry-muted3)]">
                  {t("settings.securityPasswordMinLengthHelp", {
                    min: minPasswordLength,
                  })}
                </p>
              </div>
            </section>

            <section className={`${SECURITY_INSET_CLASS} p-4`}>
              <div className="mb-2">
                <h4 className="text-sm font-semibold text-[var(--scry-ink2)]">
                  Access controls
                </h4>
              </div>
              <div className="divide-y divide-[var(--scry-line2)]">
                <CheckboxField
                  id="security-skip-local-ips"
                  checked={settings.skipLoginForLocalIps}
                  disabled={busy}
                  onCheckedChange={(checked) =>
                    onSkipLocalIpsChange(checked === true)
                  }
                  label={t("settings.securitySkipLocalIps")}
                  labelAccessory={
                    <InfoHelp
                      ariaLabel={t("settings.securitySkipLocalIps")}
                      text={t("settings.securitySkipLocalIpsHelp")}
                    />
                  }
                  className="w-full items-center py-3"
                  checkboxClassName="mt-0"
                />
                <CheckboxField
                  id="security-api-keys-restrict-to-system-settings-users"
                  checked={settings.apiKeysRestrictToSystemSettingsUsers}
                  disabled={busy || !canManageApiKeysRestriction}
                  onCheckedChange={(checked) =>
                    onApiKeysRestrictionChange(checked === true)
                  }
                  label="Restrict API keys to system-settings users"
                  labelAccessory={
                    <InfoHelp
                      ariaLabel="Restrict API keys to system-settings users"
                      text="Only users with Manage System Settings can create or use API keys. Existing keys are preserved and resume if permission is restored or this setting is disabled."
                    />
                  }
                  className="w-full items-center py-3"
                  checkboxClassName="mt-0"
                />
              </div>
            </section>

            <section className={`${SECURITY_INSET_CLASS} p-4 lg:col-span-2`}>
              <div className="mb-3 space-y-1">
                <h4 className="text-sm font-semibold text-[var(--scry-ink2)]">
                  Multi-factor authentication
                </h4>
                <p className="text-xs text-[var(--scry-muted3)]">
                  Choose where an enrolled passkey or authenticator is required.
                </p>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                <CheckboxField
                  id="security-mfa-config-step-up"
                  checked={settings.mfaRequireConfigStepUp}
                  disabled={busy}
                  onCheckedChange={(checked) =>
                    onMfaConfigStepUpChange(checked === true)
                  }
                  label={t("settings.securityMfaConfigStepUp")}
                  labelAccessory={
                    <InfoHelp
                      ariaLabel={t("settings.securityMfaConfigStepUp")}
                      text={t("settings.securityMfaConfigStepUpHelp")}
                    />
                  }
                  className="max-w-full rounded-[10px] bg-[var(--scry-inset)] px-3 py-3"
                />
                <CheckboxField
                  id="security-mfa-password-login"
                  checked={settings.mfaRequirePasswordLogin}
                  disabled={busy}
                  onCheckedChange={(checked) =>
                    onMfaPasswordLoginChange(checked === true)
                  }
                  label={t("settings.securityMfaPasswordLogin")}
                  labelAccessory={
                    <InfoHelp
                      ariaLabel={t("settings.securityMfaPasswordLogin")}
                      text={t("settings.securityMfaPasswordLoginHelp")}
                    />
                  }
                  className="max-w-full rounded-[10px] bg-[var(--scry-inset)] px-3 py-3"
                />
                <CheckboxField
                  id="security-mfa-jellyfin-login"
                  checked={settings.mfaRequireJellyfinLogin}
                  disabled={busy}
                  onCheckedChange={(checked) =>
                    onTotpJellyfinLoginChange(checked === true)
                  }
                  label={t("settings.securityTotpJellyfinLogin")}
                  labelAccessory={
                    <InfoHelp
                      ariaLabel={t("settings.securityTotpJellyfinLogin")}
                      text={t("settings.securityTotpJellyfinLoginHelp")}
                    />
                  }
                  className="max-w-full rounded-[10px] bg-[var(--scry-inset)] px-3 py-3"
                />
                <CheckboxField
                  id="security-mfa-emby-login"
                  checked={settings.mfaRequireEmbyLogin}
                  disabled={busy}
                  onCheckedChange={(checked) => onTotpEmbyLoginChange(checked === true)}
                  label="Require MFA for Emby login"
                  labelAccessory={
                    <InfoHelp
                      ariaLabel="Require MFA for Emby login"
                      text="Require an enrolled Scryer passkey or authenticator factor after either Local or Connect Emby authentication."
                    />
                  }
                  className="max-w-full rounded-[10px] bg-[var(--scry-inset)] px-3 py-3"
                />
              </div>
            </section>
          </div>
        </div>

        {settings.envOverrideActive ? (
          <div className="space-y-3 rounded-[14px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-4 shadow-[0_10px_24px_rgba(0,0,0,0.16)]">
            <div className="space-y-1">
              <h4 className="text-sm font-medium text-[var(--scry-ink2)]">{t("settings.securityOverrideTitle")}</h4>
              <p className="text-xs text-[var(--scry-muted3)]">
                {t("settings.securityOverrideDescription")}
              </p>
              {settings.envOverrideDescription ? (
                <p className="text-xs text-[var(--scry-muted3)]">
                  {t("settings.securityOverrideReason", {
                    override: settings.envOverrideDescription,
                  })}
                </p>
              ) : null}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className={`${SECURITY_INSET_CLASS} p-3`}>
                <div className="text-xs uppercase tracking-wide text-[var(--scry-muted3)]">
                  {t("settings.securitySavedPreference")}
                </div>
                <div className="mt-1 font-medium text-[var(--scry-ink2)]">
                  {settings.formLoginEnabled
                    ? t("settings.securityModeEnabled")
                    : t("settings.securityModeDisabled")}
                </div>
              </div>
              <div className={`${SECURITY_INSET_CLASS} p-3`}>
                <div className="text-xs uppercase tracking-wide text-[var(--scry-muted3)]">
                  {t("settings.securityEffectiveMode")}
                </div>
                <div className="mt-1 font-medium text-[var(--scry-ink2)]">
                  {settings.effectiveFormLoginEnabled
                    ? t("settings.securityModeEnabled")
                    : t("settings.securityModeDisabled")}
                </div>
              </div>
            </div>
          </div>
        ) : null}

        {externalAccountInvitesPanel}
        {oauthApplicationsPanel}
      </div>

      <ConfirmDialog
        open={setPasswordOpen}
        contentId="settings-security-set-password-dialog"
        title={t("settings.securitySetAdminPasswordTitle")}
        description={t("settings.securitySetAdminPasswordDescription")}
        confirmLabel={t("settings.securitySetAdminPasswordAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-set-password-confirm"
        cancelButtonId="settings-security-set-password-cancel"
        confirmButtonVariant="default"
        confirmButtonClassName="bg-[var(--scry-success-solid)] text-[var(--scry-success-on-solid)] hover:bg-[var(--scry-success-solid-hover)] focus-visible:ring-[var(--scry-success-border-strong)]"
        isBusy={confirmBusy}
        confirmDisabled={setPasswordDisabled}
        onConfirm={onConfirmSetPassword}
        onCancel={onCancelSetPassword}
      >
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="security-new-password">
              {t("profile.newPassword")}
            </Label>
            <Input
              id="security-new-password"
              type="password"
              autoComplete="new-password"
              value={newPassword}
              onChange={(event) => onNewPasswordChange(event.target.value)}
            />
            <p className="text-xs text-[var(--scry-muted3)]">
              {t("settings.securitySetAdminPasswordMinLength", {
                count: settings.passwordMinLength,
              })}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="security-new-password-confirm">
              {t("profile.confirmPassword")}
            </Label>
            <Input
              id="security-new-password-confirm"
              type="password"
              autoComplete="new-password"
              value={newPasswordConfirm}
              onChange={(event) => onNewPasswordConfirmChange(event.target.value)}
            />
          </div>
          {setPasswordError ? (
            <p id="settings-security-set-password-error" className="text-xs text-destructive">
              {setPasswordError}
            </p>
          ) : null}
        </div>
      </ConfirmDialog>

      <ConfirmDialog
        open={enableConfirmOpen}
        contentId="settings-security-enable-dialog"
        title={t("settings.securityConfirmTitle")}
        description={t("settings.securityConfirmDescription")}
        confirmLabel={t("settings.securityConfirmAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-enable-confirm"
        cancelButtonId="settings-security-enable-cancel"
        confirmButtonVariant="default"
        confirmButtonClassName="bg-[var(--scry-success-solid)] text-[var(--scry-success-on-solid)] hover:bg-[var(--scry-success-solid-hover)] focus-visible:ring-[var(--scry-success-border-strong)]"
        isBusy={confirmBusy}
        confirmDisabled={confirmDisabled}
        onConfirm={onConfirmEnable}
        onCancel={onCancelEnable}
      >
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="security-confirm-password">
              {t("settings.securityConfirmPassword")}
            </Label>
            <Input
              id="security-confirm-password"
              type="password"
              autoComplete="current-password"
              value={confirmPassword}
              onChange={(event) => onConfirmPasswordChange(event.target.value)}
            />
          </div>
          {confirmError ? (
            <p id="settings-security-confirm-error" className="text-xs text-destructive">
              {confirmError}
            </p>
          ) : null}
        </div>
      </ConfirmDialog>

      <ConfirmDialog
        open={disableConfirmOpen}
        contentId="settings-security-disable-dialog"
        title={t("settings.securityDisableConfirmTitle")}
        description={t("settings.securityDisableConfirmDescription")}
        confirmLabel={t("settings.securityDisableConfirmAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-disable-confirm"
        cancelButtonId="settings-security-disable-cancel"
        isBusy={confirmBusy}
        onConfirm={onConfirmDisable}
        onCancel={onCancelDisable}
      />
    </>
  );
}
