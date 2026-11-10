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
    <section
      aria-labelledby="settings-network-group-title"
      className="overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]"
    >
      <div className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3">
        <h3
          id="settings-network-group-title"
          className="text-[15px] font-semibold text-[var(--scry-ink2)]"
        >
          {t("settings.networkGroupTitle")}
        </h3>
      </div>
      <div className="space-y-4 p-4">
        <NetworkBindPanel onSaved={() => setRevision((value) => value + 1)} />
        <PublicUrlPanel key={revision} />
        <TrustedProxiesPanel />
      </div>
    </section>
  );
}
