import { useTranslate } from "@/lib/context/translate-context";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";

export function ListProviderSetup() {
  const t = useTranslate();
  const user = useSessionUser();
  const canManagePlugins = hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  return (
    <p className="text-sm text-[var(--scry-muted3)]">
      {t(canManagePlugins ? "lists.accounts.installProviders" : "lists.accounts.askForProviders")}
    </p>
  );
}
