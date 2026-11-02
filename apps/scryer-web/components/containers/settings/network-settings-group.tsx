import * as React from "react";
import { NetworkBindPanel } from "@/components/containers/settings/network-bind-panel";
import { PublicUrlPanel } from "@/components/containers/settings/public-url-panel";
import { TrustedProxiesPanel } from "@/components/containers/settings/trusted-proxies-panel";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";

/** How Scryer is reached: who can connect, its public address, and proxies. */
export function NetworkSettingsGroup() {
  const t = useTranslate();
  const { user } = useAuth();
  // The effective-values table reads the listening address too, so it is
  // reloaded after a network change rather than showing the old values.
  const [revision, setRevision] = React.useState(0);
  const allowed =
    user != null &&
    hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  if (!allowed) return null;
  return (
    <section aria-labelledby="settings-network-group-title" className="mb-6">
      <h2
        id="settings-network-group-title"
        className="mb-3 text-lg font-semibold"
      >
        {t("settings.networkGroupTitle")}
      </h2>
      <NetworkBindPanel onSaved={() => setRevision((value) => value + 1)} />
      <PublicUrlPanel key={revision} />
      <TrustedProxiesPanel />
    </section>
  );
}
