import { useTranslate } from "@/lib/context/translate-context";
import * as React from "react";
import { Copy, Pencil, Plus, Trash2 } from "lucide-react";
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
  canCreateJellyfinPluginClient,
  canStartJellyfinPluginClientCreation,
  createdJellyfinPluginClientForCallback,
  isEligibleJellyfinPluginClient,
  jellyfinConnectionIneligibilityReasons,
  jellyfinPluginCallbackUrl,
  jellyfinPluginClientRegistrations,
  jellyfinPluginClientStatus,
  jellyfinPluginClientCreateDecision,
  jellyfinPluginCreateNeedsReconciliation,
  normalizedPublicJellyfinBaseUrl,
  prefillJellyfinPublicBaseUrl,
  reconcileCreatedJellyfinPluginClient,
  shouldApplyJellyfinPluginOAuthReload,
  type JellyfinMediaServerConnection,
  type OAuthClientKind,
  type OAuthClientRegistrationForJellyfin,
} from "@/lib/utils/jellyfin-plugin-oauth-setup";
import { LoadingMark } from "@/components/common/loading-mark";

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
  const t = useTranslate();
  const client = useClient();
  const [clients, setClients] = React.useState<OAuthClientRegistration[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [editingClientId, setEditingClientId] = React.useState<string | null>(
    null,
  );
  const [draft, setDraft] = React.useState<OAuthClientDraft>(EMPTY_DRAFT);
  const [deleteTarget, setDeleteTarget] =
    React.useState<OAuthClientRegistration | null>(null);
  const [jellyfinPublicBaseUrl, setJellyfinPublicBaseUrl] = React.useState("");
  const [jellyfinPublicBaseUrlTouched, setJellyfinPublicBaseUrlTouched] =
    React.useState(false);
  const [createdJellyfinClient, setCreatedJellyfinClient] = React.useState<{
    clientId: string;
    callbackUrl: string;
  } | null>(null);
  const [uncertainJellyfinCallbackUrl, setUncertainJellyfinCallbackUrl] =
    React.useState<string | null>(null);
  const reloadGenerationRef = React.useRef(0);
  const jellyfinCreateKeyRef = React.useRef<string | null>(null);
  const [jellyfinConnections, setJellyfinConnections] = React.useState<
    JellyfinMediaServerConnection[] | null
  >(null);

  const reload = React.useCallback(
    async (reconcileClient?: { clientId: string; callbackUrl: string }) => {
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
            .query<{
              mediaServerConnections?: JellyfinMediaServerConnection[];
            }>(
              mediaServerConnectionsQuery,
              { provider: "JELLYFIN" },
              { requestPolicy: "network-only" },
            )
            .toPromise(),
        ]);
        if (result.error) throw result.error;
        if (
          !shouldApplyJellyfinPluginOAuthReload(
            reloadGeneration,
            reloadGenerationRef.current,
          )
        )
          return;
        const nextClients = result.data?.oauthClientRegistrations ?? [];
        setClients(nextClients);
        setUncertainJellyfinCallbackUrl(null);
        setJellyfinConnections(
          jellyfinResult.error
            ? null
            : (jellyfinResult.data?.mediaServerConnections ?? []),
        );
        setCreatedJellyfinClient((previousClient) => {
          const clientToReconcile = reconcileClient ?? previousClient;
          if (!clientToReconcile) return null;
          return reconcileCreatedJellyfinPluginClient(
            clientToReconcile,
            nextClients,
          );
        });
      } catch (error) {
        if (
          !shouldApplyJellyfinPluginOAuthReload(
            reloadGeneration,
            reloadGenerationRef.current,
          )
        )
          return;
        toast.error(
          userFacingGraphQlErrorMessage(
            error,
            t("error.fallback.unableToLoadOAuthApplications"),
            t,
          ),
        );
      } finally {
        if (
          shouldApplyJellyfinPluginOAuthReload(
            reloadGeneration,
            reloadGenerationRef.current,
          )
        )
          setLoading(false);
      }
    },
    [client, t],
  );

  React.useEffect(() => {
    void reload();
  }, [reload]);

  React.useEffect(() => {
    if (jellyfinPublicBaseUrlTouched || jellyfinPublicBaseUrl) return;
    const prefill = prefillJellyfinPublicBaseUrl(jellyfinConnections);
    if (prefill) setJellyfinPublicBaseUrl(prefill);
  }, [
    jellyfinConnections,
    jellyfinPublicBaseUrl,
    jellyfinPublicBaseUrlTouched,
  ]);

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
        editingClientId
          ? t("oauth.ui.OAuth_application_updated")
          : t("oauth.ui.OAuth_application_created"),
      );
      resetDraft();
      await reload();
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(
          error,
          t("error.fallback.unableToSaveOAuthApplication"),
          t,
        ),
      );
    } finally {
      setBusy(false);
    }
  }, [client, draft, editingClientId, reload, resetDraft, t]);

  const copyClientId = React.useCallback(async (clientId: string) => {
    if (!navigator.clipboard) {
      toast.error(t("oauth.ui.Unable_to_copy_the_OAuth_client_ID"));
      return;
    }
    try {
      await navigator.clipboard.writeText(clientId);
      toast.success(t("oauth.ui.OAuth_client_ID_copied"));
    } catch {
      toast.error(t("oauth.ui.Unable_to_copy_the_OAuth_client_ID"));
    }
  }, [t]);

  const copyJellyfinCallback = React.useCallback(
    async (callbackUrl: string) => {
      if (!navigator.clipboard) {
        toast.error(t("oauth.ui.Unable_to_copy_the_Jellyfin_callback_URL"));
        return;
      }
      try {
        await navigator.clipboard.writeText(callbackUrl);
        toast.success(t("oauth.ui.Jellyfin_callback_URL_copied"));
      } catch {
        toast.error(t("oauth.ui.Unable_to_copy_the_Jellyfin_callback_URL"));
      }
    },
    [t],
  );

  const createJellyfinPluginClient = React.useCallback(async () => {
    const publicBaseUrl = normalizedPublicJellyfinBaseUrl(
      jellyfinPublicBaseUrl,
    );
    if (!publicBaseUrl) {
      toast.error(t("oauth.ui.Enter_a_valid_public_HTTPS_Jellyfin_base_URL"));
      return;
    }
    const callbackUrl = jellyfinPluginCallbackUrl(publicBaseUrl)!;
    if (!canStartJellyfinPluginClientCreation(jellyfinCreateKeyRef.current))
      return;
    const createDecision = jellyfinPluginClientCreateDecision(
      clients,
      callbackUrl,
    );
    if (createDecision === "ambiguous") {
      toast.error(
        t(
          "oauth.ui.Multiple_custom_OAuth_clients_use_this_exact_Jellyfin_callback_Remove_duplicates_before_continuing",
        ),
      );
      return;
    }
    if (createDecision === "reuse") {
      const existingClient = clients.find((registeredClient) =>
        isEligibleJellyfinPluginClient(registeredClient, callbackUrl),
      );
      if (!existingClient) return;
      setCreatedJellyfinClient({
        clientId: existingClient.clientId,
        callbackUrl,
      });
      toast.success(
        t("oauth.ui.Jellyfin_plugin_OAuth_client_is_already_configured"),
      );
      return;
    }
    jellyfinCreateKeyRef.current = callbackUrl;
    setBusy(true);
    try {
      const result = await client
        .mutation<
          { createOAuthClientRegistration?: OAuthClientRegistration },
          {
            input: {
              displayName: string;
              redirectUris: string[];
              kind: OAuthClientKind;
            };
          }
        >(createOAuthClientRegistrationMutation, {
          input: {
            displayName: JELLYFIN_PLUGIN_DISPLAY_NAME,
            redirectUris: [callbackUrl],
            kind: "JELLYFIN_PLUGIN",
          },
        })
        .toPromise();
      const registration = result.data?.createOAuthClientRegistration;
      if (jellyfinPluginCreateNeedsReconciliation(registration)) {
        setUncertainJellyfinCallbackUrl(callbackUrl);
        toast.error(
          t(
            "oauth.ui.The_create_result_is_uncertain_Reloading_OAuth_applications_before_another_create_is_allowed",
          ),
        );
        await reload();
        return;
      }
      const createdClient = {
        clientId: registration.clientId,
        callbackUrl,
      };
      setCreatedJellyfinClient(createdClient);
      toast.success(t("oauth.ui.Jellyfin_plugin_OAuth_client_created"));
      await reload(createdClient);
    } catch (error) {
      setUncertainJellyfinCallbackUrl(callbackUrl);
      toast.error(
        userFacingGraphQlErrorMessage(
          error,
          t(
            "error.fallback.theCreateResultIsUncertainReloadOAuthApplicationsBeforeTryingAgain",
          ),
          t,
        ),
      );
      await reload();
    } finally {
      if (jellyfinCreateKeyRef.current === callbackUrl)
        jellyfinCreateKeyRef.current = null;
      setBusy(false);
    }
  }, [client, clients, jellyfinPublicBaseUrl, reload, t]);

  const deleteClient = React.useCallback(async () => {
    if (!deleteTarget) return;
    setBusy(true);
    try {
      const result = await client
        .mutation<
          { deleteOAuthClientRegistration?: { deleted: boolean } },
          { clientId: string }
        >(deleteOAuthClientRegistrationMutation, {
          clientId: deleteTarget.clientId,
        })
        .toPromise();
      if (result.error) throw result.error;
      setDeleteTarget(null);
      resetDraft();
      toast.success(
        t("oauth.ui.OAuth_application_deleted_and_its_grants_revoked"),
      );
      await reload();
    } catch (error) {
      toast.error(
        userFacingGraphQlErrorMessage(
          error,
          t("error.fallback.unableToDeleteOAuthApplication"),
          t,
        ),
      );
    } finally {
      setBusy(false);
    }
  }, [client, deleteTarget, reload, resetDraft, t]);

  const managedClients = clients.filter(
    (client) =>
      client.source === "MANAGED" && client.clientId !== "generic-native",
  );
  // A plugin client is identified by the kind stored on its registration, so it belongs to the
  // Jellyfin section rather than the generic custom-application list that created it.
  const jellyfinPluginClients = jellyfinPluginClientRegistrations(clients);
  const customClients = clients.filter(
    (client) => client.source === "CUSTOM" && client.kind === "CUSTOM",
  );
  const editingClient = clients.find(
    (client) =>
      client.source === "CUSTOM" && client.clientId === editingClientId,
  );
  const callbackPreview = normalizedPublicJellyfinBaseUrl(
    jellyfinPublicBaseUrl,
  );
  const previewCallbackUrl = jellyfinPluginCallbackUrl(callbackPreview);
  const createdClientMatchesCallback =
    createdJellyfinClient && previewCallbackUrl
      ? createdJellyfinClient.callbackUrl === previewCallbackUrl
      : false;
  const uncertainCreateMatchesCallback =
    uncertainJellyfinCallbackUrl && previewCallbackUrl
      ? uncertainJellyfinCallbackUrl === previewCallbackUrl
      : false;
  const jellyfinClientStatus = jellyfinPluginClientStatus(
    jellyfinPluginClients.length,
    Boolean(createdClientMatchesCallback),
    Boolean(uncertainCreateMatchesCallback),
  );
  const reconcilingJellyfinClient =
    jellyfinPluginClients.length === 0 && createdClientMatchesCallback
      ? createdJellyfinClient
      : null;
  // Once a plugin client exists its stored callback, not the text box, defines the public URL.
  const jellyfinPublicBaseUrlInEffect =
    jellyfinPluginClients.length === 1
      ? jellyfinPluginClients[0].publicBaseUrl
      : callbackPreview;
  const linkingStatus = automaticLinkingStatus(
    jellyfinPublicBaseUrlInEffect,
    jellyfinConnections,
  );
  const linkingIneligibilities = jellyfinConnectionIneligibilityReasons(
    jellyfinPublicBaseUrlInEffect,
    jellyfinConnections,
  );

  return (
    <div className={PANEL_CLASS}>
      <div className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <h3 className="text-[15px] font-semibold text-[var(--scry-ink2)]">
              {t("uiText.oAuthApplications")}
            </h3>
          </div>
          {loading ? (
            <LoadingMark className="h-4 w-4 text-[var(--scry-muted3)]" />
          ) : null}
        </div>
      </div>

      <div className="space-y-5 p-4">
        <section
          className={`${INSET_CLASS} space-y-3 p-3`}
          aria-labelledby="jellyfin-plugin-oauth-title"
        >
          <div className="space-y-1">
            <h4
              id="jellyfin-plugin-oauth-title"
              className="text-sm font-medium text-[var(--scry-ink2)]"
            >
              {t("uiText.jellyfinPluginOAuth")}
            </h4>
            <p className="text-xs text-[var(--scry-muted3)]">
              {t(
                "uiText.createAStandaloneOAuthClientForTheJellyfinPluginThisDoesNotRequireAJellyfinMediaServerConnectionOrAccountLinking",
              )}
            </p>
            <div className="space-y-1 text-xs text-[var(--scry-muted3)]">
              <p>
                {t("uiText.oAuthClient")}{" "}
                {jellyfinClientStatus === "ready"
                  ? "ready"
                  : jellyfinClientStatus === "reconciling"
                    ? t("oauth.ui.reconciling_the_created_client")
                    : jellyfinClientStatus === "ambiguous"
                      ? t(
                          "oauth.ui.ambiguous_multiple_custom_clients_use_this_exact_callback_remove_duplicates_before_continuing",
                        )
                      : t("oauth.ui.not_configured_for_this_callback")}
              </p>
              <p>
                {t("uiText.automaticAccountLinking")}{" "}
                {linkingStatus === "ready"
                  ? "ready"
                  : linkingStatus === "ambiguous"
                    ? t(
                        "oauth.ui.ambiguous_more_than_one_eligible_Jellyfin_connection_matches_this_public_URL",
                      )
                    : linkingStatus === "unavailable"
                      ? t("oauth.ui.connection_status_unavailable")
                      : linkingStatus === "enter-url"
                        ? t(
                            "oauth.ui.enter_a_public_HTTPS_URL_to_check_connection_binding",
                          )
                        : t(
                            "oauth.ui.requires_exactly_one_enabled_Jellyfin_connection_with_linking_an_API_key_and_this_exact_public_URL",
                          )}
              </p>
            </div>
            {linkingIneligibilities.length > 0 ? (
              <div
                id="jellyfin-plugin-linking-ineligibility"
                role="status"
                className="space-y-1 rounded-[10px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-2 text-xs text-[var(--scry-warning-text)]"
              >
                <p>{t("uiText.accountLinkingCannotUseThisPublicURL")}</p>
                <ul className="space-y-0.5">
                  {linkingIneligibilities.map((entry) => (
                    <li key={`${entry.displayName}-${entry.reason}`}>
                      {`${entry.displayName} ${entry.reason}`}
                    </li>
                  ))}
                </ul>
                <p>
                  {t(
                    "uiText.thePluginStillSignsInItGrantsLibraryAccessOnlyUntilThisIsFixed",
                  )}
                </p>
              </div>
            ) : null}
          </div>
          {jellyfinPluginClients.length > 0 ? (
            <div className="space-y-2">
              {jellyfinPluginClients.length > 1 ? (
                <p
                  id="jellyfin-plugin-client-ambiguous"
                  className="rounded-[10px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-2 text-xs text-[var(--scry-warning-text)]"
                >
                  {t(
                    "uiText.moreThanOneOAuthClientIsRegisteredAsTheJellyfinPluginDeleteAllButOneBeforeThePluginSignsIn",
                  )}
                </p>
              ) : null}
              {jellyfinPluginClients.map((registeredClient) => (
                <div
                  key={registeredClient.clientId}
                  className="space-y-2 rounded-[10px] border border-[var(--scry-line2)] p-3"
                >
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                      <div className="flex items-center gap-2">
                        <p className="text-sm font-medium text-[var(--scry-ink2)]">
                          {registeredClient.displayName}
                        </p>
                        <span className="text-xs text-[var(--scry-muted3)]">
                          {registeredClient.enabled ? "Enabled" : "Disabled"}
                        </span>
                      </div>
                      <ClientIdLine
                        clientId={registeredClient.clientId}
                        onCopy={copyClientId}
                      />
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
                        {t("label.edit")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={busy}
                        onClick={() => setDeleteTarget(registeredClient)}
                      >
                        <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                        {t("label.delete")}
                      </Button>
                    </div>
                  </div>
                  {registeredClient.callbackUrl ? (
                    <div className="flex min-w-0 items-center gap-1.5">
                      <code className="min-w-0 break-all text-xs text-[var(--scry-muted3)]">
                        {registeredClient.callbackUrl}
                      </code>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        className="shrink-0"
                        aria-label={t(
                          "uiAttr.Copy_Jellyfin_plugin_callback_URL",
                        )}
                        onClick={() =>
                          void copyJellyfinCallback(
                            registeredClient.callbackUrl!,
                          )
                        }
                      >
                        <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                      </Button>
                    </div>
                  ) : (
                    <p className="text-xs text-[var(--scry-muted3)]">
                      {t(
                        "uiText.thisClientHasNoCallbackURLRegisteredEditItToAddThePluginCallback",
                      )}
                    </p>
                  )}
                  {registeredClient.publicBaseUrl ? (
                    <p className="break-all text-xs text-[var(--scry-muted3)]">
                      {`Public Jellyfin base URL: ${registeredClient.publicBaseUrl}`}
                    </p>
                  ) : null}
                </div>
              ))}
            </div>
          ) : (
            <>
              <div className="space-y-1.5">
                <Label htmlFor="jellyfin-plugin-public-base-url">
                  {t("uiText.publicJellyfinBaseURL")}
                </Label>
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
                  {previewCallbackUrl
                    ? `Exact callback: ${previewCallbackUrl}`
                    : t(
                        "oauth.ui.Enter_a_public_HTTPS_URL_to_derive_the_exact_callback",
                      )}
                </p>
              </div>
              <Button
                type="button"
                disabled={
                  !canCreateJellyfinPluginClient(busy, jellyfinClientStatus)
                }
                onClick={() => void createJellyfinPluginClient()}
              >
                {busy ? (
                  <LoadingMark className="h-4 w-4" />
                ) : (
                  <Plus className="h-4 w-4" />
                )}
                {t("uiText.createJellyfinPluginClient")}
              </Button>
              {reconcilingJellyfinClient ? (
                <div className="space-y-2 rounded-[10px] border border-[var(--scry-line2)] p-3">
                  <p className="text-sm font-medium text-[var(--scry-ink2)]">
                    {t("uiText.jellyfinPluginOAuthClientReconciliationPending")}
                  </p>
                  <ClientIdLine
                    clientId={reconcilingJellyfinClient.clientId}
                    onCopy={copyClientId}
                  />
                  <div className="flex min-w-0 items-center gap-1.5">
                    <code className="min-w-0 break-all text-xs text-[var(--scry-muted3)]">
                      {reconcilingJellyfinClient.callbackUrl}
                    </code>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="shrink-0"
                      aria-label={t("uiAttr.Copy_Jellyfin_plugin_callback_URL")}
                      onClick={() =>
                        void copyJellyfinCallback(
                          reconcilingJellyfinClient.callbackUrl,
                        )
                      }
                    >
                      <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={loading}
                    onClick={() => void reload(reconcilingJellyfinClient)}
                  >
                    {loading ? <LoadingMark className="h-4 w-4" /> : null}
                    {t("uiText.retryClientList")}
                  </Button>
                </div>
              ) : null}
              {jellyfinClientStatus === "reconciling" &&
              !reconcilingJellyfinClient ? (
                <Button
                  type="button"
                  variant="outline"
                  disabled={loading}
                  onClick={() => void reload()}
                >
                  {loading ? <LoadingMark className="h-4 w-4" /> : null}
                  {t("uiText.retryClientList")}
                </Button>
              ) : null}
            </>
          )}
        </section>
        {managedClients.length > 0 ? (
          <div className="space-y-2">
            <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
              {t("uiText.managedApplications")}
            </h4>
            {managedClients.map((registeredClient) => (
              <div
                key={registeredClient.clientId}
                className={`${INSET_CLASS} space-y-2 p-3`}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-medium text-[var(--scry-ink2)]">
                    {registeredClient.displayName}
                  </span>
                  <span className="text-xs text-[var(--scry-muted3)]">
                    {t("uiText.managedByScryer")}
                  </span>
                </div>
                <ClientIdLine
                  clientId={registeredClient.clientId}
                  onCopy={copyClientId}
                />
                <p className="text-xs text-[var(--scry-muted3)]">
                  {registeredClient.redirectUris.length > 0
                    ? registeredClient.redirectUris.join(", ")
                    : t(
                        "oauth.ui.Uses_Scryer_s_built_in_native_callback_policy",
                      )}
                </p>
              </div>
            ))}
          </div>
        ) : null}

        <div className="space-y-2">
          <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
            {t("uiText.customApplications")}
          </h4>
          {customClients.length === 0 && !loading ? (
            <p
              className={`${INSET_CLASS} p-3 text-xs text-[var(--scry-muted3)]`}
            >
              {t("uiText.noCustomOAuthApplicationsAreRegistered")}
            </p>
          ) : null}
          {customClients.map((registeredClient) => (
            <div
              key={registeredClient.clientId}
              id={`oauth-client-${registeredClient.clientId}`}
              data-ui="oauth-client-registration"
              data-client-name={registeredClient.displayName}
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
                  <ClientIdLine
                    clientId={registeredClient.clientId}
                    onCopy={copyClientId}
                  />
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    id={`oauth-client-edit-${registeredClient.clientId}`}
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
                    {t("label.edit")}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setDeleteTarget(registeredClient)}
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                    {t("label.delete")}
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
          id="oauth-client-form"
          className={`${INSET_CLASS} space-y-4 p-3`}
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
              {editingClient
                ? `Edit ${editingClient.displayName}`
                : t("oauth.ui.Register_an_application")}
            </h4>
            {editingClient ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                id="oauth-client-cancel"
                onClick={resetDraft}
              >
                {t("label.cancel")}
              </Button>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="oauth-client-display-name">
              {t("uiText.displayName")}
            </Label>
            <Input
              id="oauth-client-display-name"
              value={draft.displayName}
              disabled={busy}
              onChange={(event) =>
                setDraft((previous) => ({
                  ...previous,
                  displayName: event.target.value,
                }))
              }
              placeholder={t("uiAttr.Example_integration")}
              required
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="oauth-client-redirect-uris">
              {t("uiText.hTTPSCallbackURLs")}
            </Label>
            <textarea
              id="oauth-client-redirect-uris"
              className="min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 font-[var(--font-code)] text-xs shadow-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
              value={draft.redirectUris}
              disabled={busy}
              onChange={(event) =>
                setDraft((previous) => ({
                  ...previous,
                  redirectUris: event.target.value,
                }))
              }
              placeholder="https://service.example/oauth/callback"
              required
            />
            <p className="text-xs text-[var(--scry-muted3)]">
              {t("uiText.oneExactHTTPSURLPerLine")}
            </p>
          </div>
          {editingClient ? (
            <CheckboxField
              id="oauth-client-enabled"
              checked={draft.enabled}
              disabled={busy}
              onCheckedChange={(checked) =>
                setDraft((previous) => ({
                  ...previous,
                  enabled: checked === true,
                }))
              }
              label={t("uiAttr.Enabled")}
              description={t(
                "uiAttr.Disabling_immediately_revokes_all_existing_grants_and_tokens_for_this_application",
              )}
              className="rounded-[10px] border border-[var(--scry-line2)] px-3 py-2"
            />
          ) : null}
          <Button id="oauth-client-save" type="submit" disabled={busy}>
            {busy ? (
              <LoadingMark className="h-4 w-4" />
            ) : (
              <Plus className="h-4 w-4" />
            )}
            {editingClient
              ? t("oauth.ui.Save_application")
              : t("oauth.ui.Create_application")}
          </Button>
        </form>
      </div>

      <ConfirmDialog
        open={deleteTarget != null}
        contentId="oauth-client-delete-dialog"
        title={t("uiAttr.Delete_OAuth_application")}
        description={t(
          "uiAttr.Deleting_this_application_immediately_revokes_its_existing_OAuth_grants_and_tokens_This_cannot_be_undone",
        )}
        confirmLabel={t("oauth.ui.Delete_application")}
        cancelLabel={t("uiAttr.Cancel")}
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
  const t = useTranslate();
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <code className="min-w-0 truncate text-xs text-[var(--scry-muted3)]">
        {clientId}
      </code>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        className="shrink-0"
        aria-label={t("uiAttr.Copy_OAuth_client_ID")}
        onClick={() => void onCopy(clientId)}
      >
        <Copy className="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
    </div>
  );
}
