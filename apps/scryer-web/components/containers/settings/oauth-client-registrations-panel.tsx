import * as React from "react";
import { Copy, Loader2, Pencil, Plus, Trash2 } from "lucide-react";
import { useClient } from "urql";
import { toast } from "sonner";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { CheckboxField } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  createOAuthClientRegistrationMutation,
  deleteOAuthClientRegistrationMutation,
  updateOAuthClientRegistrationMutation,
} from "@/lib/graphql/mutations";
import {
  mediaServerConnectionsQuery,
  oauthClientRegistrationsQuery,
} from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  automaticLinkingStatus,
  canStartJellyfinPluginClientCreation,
  createdJellyfinPluginClientForCallback,
  isEligibleJellyfinPluginClient,
  jellyfinPluginCallbackUrl,
  jellyfinPluginClientCreateDecision,
  normalizedPublicJellyfinBaseUrl,
  prefillJellyfinPublicBaseUrl,
  reconcileCreatedJellyfinPluginClient,
  shouldApplyJellyfinPluginOAuthReload,
  type JellyfinMediaServerConnection,
  type OAuthClientRegistrationForJellyfin,
} from "@/lib/utils/jellyfin-plugin-oauth-setup";

type OAuthClientRegistration = OAuthClientRegistrationForJellyfin & {
  displayName: string;
};

type OAuthClientDraft = {
  displayName: string;
  redirectUris: string;
  enabled: boolean;
};

const EMPTY_DRAFT: OAuthClientDraft = {
  displayName: "",
  redirectUris: "",
  enabled: true,
};

const JELLYFIN_PLUGIN_DISPLAY_NAME = "Jellyfin Scryer plugin";

const PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";

function redirectUrisFromDraft(value: string) {
  return value
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function draftFromClient(client: OAuthClientRegistration): OAuthClientDraft {
  return {
    displayName: client.displayName,
    redirectUris: client.redirectUris.join("\n"),
    enabled: client.enabled,
  };
}

export function OAuthClientRegistrationsPanel() {
  const client = useClient();
  const [clients, setClients] = React.useState<OAuthClientRegistration[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [editingClientId, setEditingClientId] = React.useState<string | null>(null);
  const [draft, setDraft] = React.useState<OAuthClientDraft>(EMPTY_DRAFT);
  const [deleteTarget, setDeleteTarget] = React.useState<OAuthClientRegistration | null>(null);
  const [jellyfinPublicBaseUrl, setJellyfinPublicBaseUrl] = React.useState("");
  const [jellyfinPublicBaseUrlTouched, setJellyfinPublicBaseUrlTouched] = React.useState(false);
  const [createdJellyfinClient, setCreatedJellyfinClient] = React.useState<{
    clientId: string;
    callbackUrl: string;
  } | null>(null);
  const reloadGenerationRef = React.useRef(0);
  const jellyfinCreateKeyRef = React.useRef<string | null>(null);
  const [jellyfinConnections, setJellyfinConnections] = React.useState<
    JellyfinMediaServerConnection[] | null
  >(null);

  const reload = React.useCallback(async (reconcileClient?: { clientId: string; callbackUrl: string }) => {
    const reloadGeneration = ++reloadGenerationRef.current;
    setLoading(true);
    try {
      const [result, jellyfinResult] = await Promise.all([
        client
          .query<{ oauthClientRegistrations?: OAuthClientRegistration[] }>(
            oauthClientRegistrationsQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise(),
        client
          .query<{ mediaServerConnections?: JellyfinMediaServerConnection[] }>(
            mediaServerConnectionsQuery,
            { provider: "JELLYFIN" },
            { requestPolicy: "network-only" },
          )
          .toPromise(),
      ]);
      if (result.error) throw result.error;
      if (!shouldApplyJellyfinPluginOAuthReload(
        reloadGeneration,
        reloadGenerationRef.current,
      )) return;
      const nextClients = result.data?.oauthClientRegistrations ?? [];
      setClients(nextClients);
      setJellyfinConnections(
        jellyfinResult.error ? null : (jellyfinResult.data?.mediaServerConnections ?? []),
      );
      setCreatedJellyfinClient((previousClient) => {
        const clientToReconcile = reconcileClient ?? previousClient;
        if (!clientToReconcile) return null;
        return reconcileCreatedJellyfinPluginClient(clientToReconcile, nextClients);
      });
    } catch (error) {
      if (!shouldApplyJellyfinPluginOAuthReload(
        reloadGeneration,
        reloadGenerationRef.current,
      )) return;
      toast.error(
        userFacingGraphQlErrorMessage(error, "Unable to load OAuth applications."),
      );
    } finally {
      if (shouldApplyJellyfinPluginOAuthReload(
        reloadGeneration,
        reloadGenerationRef.current,
      )) setLoading(false);
    }
  }, [client]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  React.useEffect(() => {
    if (jellyfinPublicBaseUrlTouched || jellyfinPublicBaseUrl) return;
    const prefill = prefillJellyfinPublicBaseUrl(jellyfinConnections);
    if (prefill) setJellyfinPublicBaseUrl(prefill);
  }, [jellyfinConnections, jellyfinPublicBaseUrl, jellyfinPublicBaseUrlTouched]);

  React.useEffect(() => {
    const callbackUrl = jellyfinPluginCallbackUrl(
      normalizedPublicJellyfinBaseUrl(jellyfinPublicBaseUrl),
    );
    setCreatedJellyfinClient((previousClient) =>
      createdJellyfinPluginClientForCallback(previousClient, callbackUrl),
    );
  }, [jellyfinPublicBaseUrl]);

  const resetDraft = React.useCallback(() => {
    setEditingClientId(null);
    setDraft(EMPTY_DRAFT);
  }, []);

  const save = React.useCallback(async () => {
    setBusy(true);
    try {
      const input = {
        displayName: draft.displayName,
        redirectUris: redirectUrisFromDraft(draft.redirectUris),
        ...(editingClientId ? { enabled: draft.enabled } : {}),
      };
      const result = editingClientId
        ? await client
            .mutation<
              { updateOAuthClientRegistration?: OAuthClientRegistration },
              { clientId: string; input: typeof input & { enabled: boolean } }
            >(updateOAuthClientRegistrationMutation, {
              clientId: editingClientId,
              input: { ...input, enabled: draft.enabled },
            })
            .toPromise()
        : await client
            .mutation<
              { createOAuthClientRegistration?: OAuthClientRegistration },
              { input: Omit<typeof input, "enabled"> }
            >(createOAuthClientRegistrationMutation, { input })
            .toPromise();
      if (result.error) throw result.error;
      toast.success(
        editingClientId ? "OAuth application updated." : "OAuth application created.",
      );
      resetDraft();
      await reload();
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(error, "Unable to save OAuth application."),
      );
    } finally {
      setBusy(false);
    }
  }, [client, draft, editingClientId, reload, resetDraft]);

  const copyClientId = React.useCallback(async (clientId: string) => {
    if (!navigator.clipboard) {
      toast.error("Unable to copy the OAuth client ID.");
      return;
    }
    try {
      await navigator.clipboard.writeText(clientId);
      toast.success("OAuth client ID copied.");
    } catch {
      toast.error("Unable to copy the OAuth client ID.");
    }
  }, []);

  const copyJellyfinCallback = React.useCallback(async (callbackUrl: string) => {
    if (!navigator.clipboard) {
      toast.error("Unable to copy the Jellyfin callback URL.");
      return;
    }
    try {
      await navigator.clipboard.writeText(callbackUrl);
      toast.success("Jellyfin callback URL copied.");
    } catch {
      toast.error("Unable to copy the Jellyfin callback URL.");
    }
  }, []);

  const createJellyfinPluginClient = React.useCallback(async () => {
    const publicBaseUrl = normalizedPublicJellyfinBaseUrl(jellyfinPublicBaseUrl);
    if (!publicBaseUrl) {
      toast.error("Enter a valid public HTTPS Jellyfin base URL.");
      return;
    }
    const callbackUrl = jellyfinPluginCallbackUrl(publicBaseUrl)!;
    if (!canStartJellyfinPluginClientCreation(jellyfinCreateKeyRef.current)) return;
    const createDecision = jellyfinPluginClientCreateDecision(clients, callbackUrl);
    if (createDecision === "ambiguous") {
      toast.error("Multiple custom OAuth clients use this exact Jellyfin callback. Remove duplicates before continuing.");
      return;
    }
    if (createDecision === "reuse") {
      const existingClient = clients.find((registeredClient) =>
        isEligibleJellyfinPluginClient(registeredClient, callbackUrl),
      );
      if (!existingClient) return;
      setCreatedJellyfinClient({ clientId: existingClient.clientId, callbackUrl });
      toast.success("Jellyfin plugin OAuth client is already configured.");
      return;
    }
    jellyfinCreateKeyRef.current = callbackUrl;
    setBusy(true);
    try {
      const result = await client
        .mutation<
          { createOAuthClientRegistration?: OAuthClientRegistration },
          { input: { displayName: string; redirectUris: string[] } }
        >(createOAuthClientRegistrationMutation, {
          input: { displayName: JELLYFIN_PLUGIN_DISPLAY_NAME, redirectUris: [callbackUrl] },
        })
        .toPromise();
      if (result.error || !result.data?.createOAuthClientRegistration) {
        throw result.error ?? new Error("Jellyfin OAuth client was not created.");
      }
      const createdClient = {
        clientId: result.data.createOAuthClientRegistration.clientId,
        callbackUrl,
      };
      setCreatedJellyfinClient(createdClient);
      toast.success("Jellyfin plugin OAuth client created.");
      await reload(createdClient);
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(error, "Unable to create the Jellyfin plugin OAuth client."),
      );
    } finally {
      if (jellyfinCreateKeyRef.current === callbackUrl) jellyfinCreateKeyRef.current = null;
      setBusy(false);
    }
  }, [client, clients, jellyfinPublicBaseUrl, reload]);

  const deleteClient = React.useCallback(async () => {
    if (!deleteTarget) return;
    setBusy(true);
    try {
      const result = await client
        .mutation<
          { deleteOAuthClientRegistration?: { deleted: boolean } },
          { clientId: string }
        >(deleteOAuthClientRegistrationMutation, { clientId: deleteTarget.clientId })
        .toPromise();
      if (result.error) throw result.error;
      setDeleteTarget(null);
      resetDraft();
      toast.success("OAuth application deleted and its grants revoked.");
      await reload();
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(error, "Unable to delete OAuth application."),
      );
    } finally {
      setBusy(false);
    }
  }, [client, deleteTarget, reload, resetDraft]);

  const managedClients = clients.filter(
    (client) => client.source === "MANAGED" && client.clientId !== "generic-native",
  );
  const customClients = clients.filter((client) => client.source === "CUSTOM");
  const editingClient = customClients.find((client) => client.clientId === editingClientId);
  const callbackPreview = normalizedPublicJellyfinBaseUrl(jellyfinPublicBaseUrl);
  const matchingJellyfinClients = callbackPreview
    ? customClients.filter((registeredClient) =>
        isEligibleJellyfinPluginClient(
          registeredClient,
          `${callbackPreview}/Scryer/Auth/Callback`,
        ),
      )
    : [];
  const createdClientMatchesCallback = createdJellyfinClient && callbackPreview
    ? createdJellyfinClient.callbackUrl === `${callbackPreview}/Scryer/Auth/Callback`
    : false;
  const jellyfinPluginClient = matchingJellyfinClients.length === 1
    ? { clientId: matchingJellyfinClients[0].clientId, callbackUrl: `${callbackPreview}/Scryer/Auth/Callback` }
    : matchingJellyfinClients.length === 0 && createdClientMatchesCallback
      ? createdJellyfinClient
      : null;
  const jellyfinClientStatus = matchingJellyfinClients.length > 1
    ? "ambiguous"
    : matchingJellyfinClients.length === 1
      ? "ready"
      : createdClientMatchesCallback
        ? "reconciling"
        : "not-configured";
  const linkingStatus = automaticLinkingStatus(callbackPreview, jellyfinConnections);

  return (
    <div className={PANEL_CLASS}>
      <div className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <h3 className="text-[15px] font-semibold text-[var(--scry-ink2)]">
              OAuth applications
            </h3>
          </div>
          {loading ? <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-muted3)]" /> : null}
        </div>
      </div>

      <div className="space-y-5 p-4">
        <section className={`${INSET_CLASS} space-y-3 p-3`} aria-labelledby="jellyfin-plugin-oauth-title">
          <div className="space-y-1">
            <h4 id="jellyfin-plugin-oauth-title" className="text-sm font-medium text-[var(--scry-ink2)]">
              Jellyfin plugin OAuth
            </h4>
            <p className="text-xs text-[var(--scry-muted3)]">
              Create a standalone OAuth client for the Jellyfin plugin. This does not require a Jellyfin media-server connection or account linking.
            </p>
            <div className="space-y-1 text-xs text-[var(--scry-muted3)]">
              <p>
                OAuth client: {jellyfinClientStatus === "ready"
                  ? "ready"
                  : jellyfinClientStatus === "reconciling"
                    ? "reconciling the created client"
                    : jellyfinClientStatus === "ambiguous"
                      ? "ambiguous: multiple custom clients use this exact callback; remove duplicates before continuing"
                      : "not configured for this callback"}
              </p>
              <p>
                Automatic account linking: {linkingStatus === "ready"
                  ? "ready"
                  : linkingStatus === "ambiguous"
                    ? "ambiguous: more than one eligible Jellyfin connection matches this public URL"
                    : linkingStatus === "unavailable"
                      ? "connection status unavailable"
                      : linkingStatus === "enter-url"
                        ? "enter a public HTTPS URL to check connection binding"
                        : "requires exactly one enabled Jellyfin connection with linking, an API key, and this exact public URL"}
              </p>
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="jellyfin-plugin-public-base-url">Public Jellyfin base URL</Label>
            <Input
              id="jellyfin-plugin-public-base-url"
              value={jellyfinPublicBaseUrl}
              disabled={busy}
              onChange={(event) => {
                setJellyfinPublicBaseUrlTouched(true);
                setJellyfinPublicBaseUrl(event.target.value);
              }}
              placeholder="https://jellyfin.example.com"
            />
            <p className="text-xs text-[var(--scry-muted3)]">
              {callbackPreview
                ? `Exact callback: ${callbackPreview}/Scryer/Auth/Callback`
                : "Enter a public HTTPS URL to derive the exact callback."}
            </p>
          </div>
          <Button
            type="button"
            disabled={busy || jellyfinClientStatus === "ambiguous"}
            onClick={() => void createJellyfinPluginClient()}
          >
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
            Create Jellyfin plugin client
          </Button>
          {jellyfinPluginClient ? (
            <div className="space-y-2 rounded-[10px] border border-[var(--scry-line2)] p-3">
              <p className="text-sm font-medium text-[var(--scry-ink2)]">
                {jellyfinClientStatus === "ready"
                  ? "Jellyfin plugin OAuth client ready"
                  : "Jellyfin plugin OAuth client reconciliation pending"}
              </p>
              <ClientIdLine clientId={jellyfinPluginClient.clientId} onCopy={copyClientId} />
              <div className="flex min-w-0 items-center gap-1.5">
                <code className="min-w-0 break-all text-xs text-[var(--scry-muted3)]">{jellyfinPluginClient.callbackUrl}</code>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  className="shrink-0"
                  aria-label="Copy Jellyfin plugin callback URL"
                  onClick={() => void copyJellyfinCallback(jellyfinPluginClient.callbackUrl)}
                >
                  <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                </Button>
              </div>
            </div>
          ) : null}
        </section>
        {managedClients.length > 0 ? (
          <div className="space-y-2">
            <h4 className="text-sm font-medium text-[var(--scry-ink2)]">Managed applications</h4>
            {managedClients.map((registeredClient) => (
              <div key={registeredClient.clientId} className={`${INSET_CLASS} space-y-2 p-3`}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-medium text-[var(--scry-ink2)]">
                    {registeredClient.displayName}
                  </span>
                  <span className="text-xs text-[var(--scry-muted3)]">Managed by Scryer</span>
                </div>
                <ClientIdLine clientId={registeredClient.clientId} onCopy={copyClientId} />
                <p className="text-xs text-[var(--scry-muted3)]">
                  {registeredClient.redirectUris.length > 0
                    ? registeredClient.redirectUris.join(", ")
                    : "Uses Scryer’s built-in native callback policy."}
                </p>
              </div>
            ))}
          </div>
        ) : null}

        <div className="space-y-2">
          <h4 className="text-sm font-medium text-[var(--scry-ink2)]">Custom applications</h4>
          {customClients.length === 0 && !loading ? (
            <p className={`${INSET_CLASS} p-3 text-xs text-[var(--scry-muted3)]`}>
              No custom OAuth applications are registered.
            </p>
          ) : null}
          {customClients.map((registeredClient) => (
            <div
              key={registeredClient.clientId}
              className={`${INSET_CLASS} space-y-3 p-3 ${registeredClient.enabled ? "" : "opacity-70"}`}
            >
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-[var(--scry-ink2)]">
                      {registeredClient.displayName}
                    </span>
                    <span className="text-xs text-[var(--scry-muted3)]">
                      {registeredClient.enabled ? "Enabled" : "Disabled"}
                    </span>
                  </div>
                  <ClientIdLine clientId={registeredClient.clientId} onCopy={copyClientId} />
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => {
                      setEditingClientId(registeredClient.clientId);
                      setDraft(draftFromClient(registeredClient));
                    }}
                  >
                    <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
                    Edit
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setDeleteTarget(registeredClient)}
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                    Delete
                  </Button>
                </div>
              </div>
              <ul className="space-y-1 break-all font-[var(--font-code)] text-xs text-[var(--scry-muted3)]">
                {registeredClient.redirectUris.map((uri) => (
                  <li key={uri}>{uri}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <form
          className={`${INSET_CLASS} space-y-4 p-3`}
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
              {editingClient ? `Edit ${editingClient.displayName}` : "Register an application"}
            </h4>
            {editingClient ? (
              <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={resetDraft}>
                Cancel
              </Button>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="oauth-client-display-name">Display name</Label>
            <Input
              id="oauth-client-display-name"
              value={draft.displayName}
              disabled={busy}
              onChange={(event) =>
                setDraft((previous) => ({ ...previous, displayName: event.target.value }))
              }
              placeholder="Example integration"
              required
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="oauth-client-redirect-uris">HTTPS callback URLs</Label>
            <textarea
              id="oauth-client-redirect-uris"
              className="min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 font-[var(--font-code)] text-xs shadow-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
              value={draft.redirectUris}
              disabled={busy}
              onChange={(event) =>
                setDraft((previous) => ({ ...previous, redirectUris: event.target.value }))
              }
              placeholder="https://service.example/oauth/callback"
              required
            />
            <p className="text-xs text-[var(--scry-muted3)]">One exact HTTPS URL per line.</p>
          </div>
          {editingClient ? (
            <CheckboxField
              id="oauth-client-enabled"
              checked={draft.enabled}
              disabled={busy}
              onCheckedChange={(checked) =>
                setDraft((previous) => ({ ...previous, enabled: checked === true }))
              }
              label="Enabled"
              description="Disabling immediately revokes all existing grants and tokens for this application."
              className="rounded-[10px] border border-[var(--scry-line2)] px-3 py-2"
            />
          ) : null}
          <Button type="submit" disabled={busy}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
            {editingClient ? "Save application" : "Create application"}
          </Button>
        </form>
      </div>

      <ConfirmDialog
        open={deleteTarget != null}
        contentId="oauth-client-delete-dialog"
        title="Delete OAuth application?"
        description="Deleting this application immediately revokes its existing OAuth grants and tokens. This cannot be undone."
        confirmLabel="Delete application"
        cancelLabel="Cancel"
        confirmButtonVariant="destructive"
        isBusy={busy}
        onConfirm={deleteClient}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}

function ClientIdLine({
  clientId,
  onCopy,
}: {
  clientId: string;
  onCopy: (clientId: string) => Promise<void>;
}) {
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <code className="min-w-0 truncate text-xs text-[var(--scry-muted3)]">{clientId}</code>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        className="shrink-0"
        aria-label="Copy OAuth client ID"
        onClick={() => void onCopy(clientId)}
      >
        <Copy className="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
    </div>
  );
}
