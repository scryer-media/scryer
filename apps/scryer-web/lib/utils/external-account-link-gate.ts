/**
 * Whether a Jellyfin account-link request is complete enough to submit.
 *
 * Password is intentionally absent. Jellyfin allows passwordless accounts, and
 * linking one is supported so its identity, watch data, and lists can be pulled.
 * An empty password only ever verifies against a genuinely passwordless account,
 * because Jellyfin returns 401 for an empty `Pw` on any account that has one.
 *
 * Refusing to *sign in* with a passwordless Jellyfin account is a separate
 * concern, enforced server-side in `federated_login_with_jellyfin`. Do not
 * reintroduce a password requirement here to try to cover that.
 */
export function canSubmitJellyfinLink({
  connectionId,
  username,
  busy,
}: {
  connectionId: string | null | undefined;
  username: string;
  busy: boolean;
}): boolean {
  return Boolean(connectionId) && username.trim().length > 0 && !busy;
}

export function effectiveEmbyLinkMode(
  requestedMode: "LOCAL" | "CONNECT",
  connectEnabled: boolean,
): "LOCAL" | "CONNECT" {
  return requestedMode === "CONNECT" && connectEnabled ? "CONNECT" : "LOCAL";
}

/**
 * Whether the link form should ask the server for the connection's accounts.
 *
 * Only someone who manages media-server connections may list a server's
 * accounts, so everyone else keeps the free-text username without a request
 * that would only be refused. Emby Connect names a Connect account, not a
 * server account, so it keeps free text too.
 */
export function shouldListServerAccounts({
  provider,
  embyMode,
  connectionId,
  canManageConnections,
}: {
  provider: "JELLYFIN" | "EMBY" | "PLEX" | null;
  embyMode: "LOCAL" | "CONNECT";
  connectionId: string | null | undefined;
  canManageConnections: boolean;
}): boolean {
  if (!canManageConnections || !connectionId) {
    return false;
  }
  return (
    provider === "JELLYFIN" || (provider === "EMBY" && embyMode === "LOCAL")
  );
}

export type ServerAccountOption = {
  value: string;
  label: string;
  linked: boolean;
};

/**
 * The username choices for a listed server, or `null` when the form should
 * keep its free-text field: the list did not load, or it offers nothing that
 * can be picked. An account already linked is shown but cannot be chosen.
 * Picking a name only fills the username; the password still proves it.
 */
export function serverAccountOptions(
  accounts: readonly { username: string; linked: boolean }[] | null,
): ServerAccountOption[] | null {
  if (!accounts) {
    return null;
  }
  const seen = new Set<string>();
  const options: ServerAccountOption[] = [];
  for (const account of accounts) {
    const username = account.username.trim();
    if (!username || seen.has(username)) {
      continue;
    }
    seen.add(username);
    options.push({ value: username, label: username, linked: account.linked });
  }
  return options.some((option) => !option.linked) ? options : null;
}

/**
 * The username to keep once a server's choices arrive: one that is not a
 * pickable choice would sit in the draft without showing in the picker.
 */
export function selectableUsername(
  username: string,
  options: readonly ServerAccountOption[] | null,
): string {
  if (!options) {
    return username;
  }
  const trimmed = username.trim();
  return options.some((option) => option.value === trimmed && !option.linked)
    ? trimmed
    : "";
}
