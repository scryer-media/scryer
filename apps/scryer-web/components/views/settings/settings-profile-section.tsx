import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";

type Props = {
  username?: string;
  currentPassword: string;
  newPassword: string;
  confirmPassword: string;
  saving: boolean;
  onCurrentPasswordChange: (value: string) => void;
  onNewPasswordChange: (value: string) => void;
  onConfirmPasswordChange: (value: string) => void;
  onChangePassword: () => void;
};

export function SettingsProfileSection({
  username,
  currentPassword,
  newPassword,
  confirmPassword,
  saving,
  onCurrentPasswordChange,
  onNewPasswordChange,
  onConfirmPasswordChange,
  onChangePassword,
}: Props) {
  const t = useTranslate();
  const passwordMismatch = confirmPassword.length > 0 && newPassword !== confirmPassword;
  const canSubmit = currentPassword.length > 0 && newPassword.length > 0 && !passwordMismatch && !saving;

  return (
    <div className="space-y-6 text-sm">
      <div className="space-y-2">
        <h3 className="text-base font-medium">{t("profile.accountInfo")}</h3>
        <div className="flex items-center gap-2 text-muted-foreground">
          <span>{t("settings.username")}:</span>
          <span className="font-medium text-foreground">{username ?? "—"}</span>
        </div>
      </div>

      <Separator />

      <div className="space-y-4">
        <h3 className="text-base font-medium">{t("profile.changePassword")}</h3>
        <div className="grid max-w-sm gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="current-password">{t("profile.currentPassword")}</Label>
            <Input
              id="current-password"
              type="password"
              autoComplete="current-password"
              value={currentPassword}
              onChange={(e) => onCurrentPasswordChange(e.target.value)}
            />
          </div>
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
            <Label htmlFor="confirm-password">{t("profile.confirmPassword")}</Label>
            <Input
              id="confirm-password"
              type="password"
              autoComplete="new-password"
              value={confirmPassword}
              onChange={(e) => onConfirmPasswordChange(e.target.value)}
            />
            {passwordMismatch ? (
              <p className="text-xs text-destructive">{t("profile.passwordMismatch")}</p>
            ) : null}
          </div>
          <Button onClick={onChangePassword} disabled={!canSubmit} className="w-fit">
            {saving ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            {t("profile.changePassword")}
          </Button>
        </div>
      </div>

      <Separator />

      <div className="space-y-2">
        <h3 className="text-base font-medium">{t("profile.externalConnections")}</h3>
        <p className="text-muted-foreground">
          {t("profile.externalConnectionsPlaceholder")}
        </p>
      </div>
    </div>
  );
}
