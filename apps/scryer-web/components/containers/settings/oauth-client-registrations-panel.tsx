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
import { oauthClientRegistrationsQuery } from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";

type OAuthClientRegistration = {
  clientId: string;
  displayName: string;
  redirectUris: string[];
  enabled: boolean;
  source: "MANAGED" | "CUSTOM";
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

  const reload = React.useCallback(async () => {
    setLoading(true);
    try {
      const result = await client
        .query<{ oauthClientRegistrations?: OAuthClientRegistration[] }>(
          oauthClientRegistrationsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (result.error) throw result.error;
      setClients(result.data?.oauthClientRegistrations ?? []);
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(error, "Unable to load OAuth applications."),
      );
    } finally {
      setLoading(false);
    }
  }, [client]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

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
