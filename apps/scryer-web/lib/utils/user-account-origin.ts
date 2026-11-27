import type { ExternalAccountProvider } from "../types/settings.ts";
import type { UserAccountKind } from "../types/users.ts";

export type UserAccountBadge = {
  /** Where the account came from, or a media server it is linked to. */
  kind: "origin" | "linked";
  label: string;
};

/**
 * The badges Known users shows for one account: where it was created, then
 * each media server it is linked to. An account a media-server login created
 * names that server as its origin rather than as a separate link.
 */
export function userAccountBadges(
  user: {
    accountKind: UserAccountKind;
    linkedMediaServers?: readonly ExternalAccountProvider[] | null;
  },
  translate: (key: string, values?: Record<string, string>) => string,
  providerLabel: (provider: ExternalAccountProvider) => string,
): UserAccountBadge[] {
  const linked = [...(user.linkedMediaServers ?? [])];
  const badges: UserAccountBadge[] = [];
  if (user.accountKind === "EXTERNAL_AUTO_PROVISIONED") {
    const origin = linked.shift();
    badges.push({
      kind: "origin",
      label: origin
        ? translate("settings.accountOrigin.mediaServerLogin", {
            provider: providerLabel(origin),
          })
        : translate("settings.accountOrigin.mediaServerLoginUnknown"),
    });
  } else {
    badges.push({
      kind: "origin",
      label: translate("settings.accountOrigin.scryer"),
    });
  }
  for (const provider of linked) {
    badges.push({
      kind: "linked",
      label: translate("settings.linkedMediaServer", {
        provider: providerLabel(provider),
      }),
    });
  }
  return badges;
}
