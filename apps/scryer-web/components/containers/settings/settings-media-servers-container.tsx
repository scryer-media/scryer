import { type ComponentProps, type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsMediaServersSection } from "@/components/views/settings/settings-media-servers-section";
import {
  createMediaServerConnectionMutation,
  deleteMediaServerConnectionMutation,
  discoverEmbyConnectServersMutation,
  discoverPlexMediaServersMutation,
  testEmbyConnectMutation,
  testMediaServerConnectionMutation,
  updateMediaServerConnectionMutation,
} from "@/lib/graphql/mutations";
import {
  authRuntimeStateQuery,
  librariesQuery,
  mediaServerConnectionsQuery,
} from "@/lib/graphql/queries";
import { useClient } from "urql";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "@/lib/utils/connection-feedback";
import { notifyExternalAccountInviteSourcesChanged } from "@/components/containers/settings/external-account-invites-container";
import { isVisibleMediaServerProvider } from "@/lib/constants/integration-providers";
import type {
  LibraryRecord,
  EmbyConnectServer,
  MediaServerConnection,
  MediaServerConnectionDraft,
  MediaServerPathMapping,
  PlexServerDiscovery,
} from "@/lib/types";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";
import { normalizeLibraryPermissionsForStorage } from "@/lib/utils/permissions";
import {
  isLocalPathFormatValidForStyle,
  localPathStyleFromRuntimeValue,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";
import { nonEmptySecret } from "@/lib/utils/secret-input";
import { normalizeMediaServerConnectionDraft } from "@/lib/utils/media-server-input";

type SettingsMediaServersSectionProps = ComponentProps<typeof SettingsMediaServersSection>;

type PendingMediaServerEditorAction =
  | { type: "create" }
  | { type: "edit"; connection: MediaServerConnection }
  | { type: "close" }
  | null;

const DEFAULT_MEDIA_SERVER_DRAFT: MediaServerConnectionDraft = {
  provider: "JELLYFIN",
  displayName: "Jellyfin",
  baseUrl: "",
  externalUrl: "",
  enabled: true,
  loginEnabled: false,
  linkingEnabled: false,
  autoAddEnabled: false,
  defaultAppPermissions: [],
  defaultLibraryGrants: [],
  machineIdPresent: false,
  plexServerId: "",
  apiKey: "",
  clearApiKey: false,
  jellyfinCredentialMode: "adminLogin",
  embyConnectionMode: "LOCAL",
  embyLocalSetupMethod: "ADMIN_CREDENTIALS",
  embyConnectEnabled: false,
  embyConnectUsernameOrEmail: "",
  embyConnectPassword: "",
  embyConnectServerId: "",
  embyDiscoveredServers: [],
  adminUsername: "",
  adminPassword: "",
  pathMappingsText: "",
};

function cloneDraft(draft: MediaServerConnectionDraft): MediaServerConnectionDraft {
  return {
    ...draft,
    defaultAppPermissions: [...draft.defaultAppPermissions],
    embyDiscoveredServers: [...draft.embyDiscoveredServers],
    defaultLibraryGrants: draft.defaultLibraryGrants.map((grant) => ({
      libraryId: grant.libraryId,
      permissions: [...grant.permissions],
    })),
  };
}

function normalizeOptional(value: string): string | undefined {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function serializePathMappings(mappings: MediaServerPathMapping[]): string {
  return mappings
    .map((mapping) => `${mapping.sourcePath.trim()} => ${mapping.destinationPath.trim()}`)
    .filter((line) => line !== "=>")
    .join("\n");
}

function parsePathMappings(value: string): MediaServerPathMapping[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [sourcePath, destinationPath = ""] = line.split(/=>/, 2);
      return {
        sourcePath: sourcePath.trim(),
        destinationPath: destinationPath.trim(),
      };
    })
    .filter((mapping) => mapping.sourcePath.length > 0 && mapping.destinationPath.length > 0);
}

function pathMappingsTextHasValidLocalPaths(
  value: string,
  localPathStyle: LocalPathStyle | undefined,
): boolean {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .every((line) => {
      const [sourcePath, destinationPath = ""] = line.split(/=>/, 2);
      const remotePath = sourcePath.trim();
      const localPath = destinationPath.trim();
      return (
        remotePath.length > 0 &&
        localPath.length > 0 &&
        isLocalPathFormatValidForStyle(localPath, localPathStyle)
      );
    });
}

function normalizeDefaultLibraryGrants(
  grants: MediaServerConnectionDraft["defaultLibraryGrants"],
): MediaServerConnectionDraft["defaultLibraryGrants"] {
  return grants
    .map((grant) => ({
      libraryId: grant.libraryId,
      permissions: normalizeLibraryPermissionsForStorage(grant.permissions),
    }))
    .filter((grant) => grant.permissions.length > 0);
}

function draftFromConnection(connection: MediaServerConnection): MediaServerConnectionDraft {
  return {
    provider: connection.provider,
    displayName: connection.displayName,
    baseUrl: connection.baseUrl,
    externalUrl: connection.externalUrl ?? "",
    enabled: connection.enabled,
    loginEnabled: connection.loginEnabled,
    linkingEnabled: connection.linkingEnabled,
    autoAddEnabled: connection.autoAddEnabled,
    defaultAppPermissions: [...connection.defaultAppPermissions],
    defaultLibraryGrants: connection.defaultLibraryGrants.map((grant) => ({
      libraryId: grant.libraryId,
      permissions: normalizeLibraryPermissionsForStorage(grant.permissions),
    })),
    machineIdPresent: connection.machineIdPresent,
    plexServerId: "",
    apiKey: "",
    clearApiKey: false,
    jellyfinCredentialMode: "apiKey",
    embyConnectionMode: "LOCAL",
    embyLocalSetupMethod: "API_KEY",
    embyConnectEnabled: connection.embyConnectEnabled,
    embyConnectUsernameOrEmail: "",
    embyConnectPassword: "",
    embyConnectServerId: "",
    embyDiscoveredServers: [],
    adminUsername: "",
    adminPassword: "",
    pathMappingsText: serializePathMappings(connection.pathMappings),
  };
}

function buildCreateInput(
  draft: MediaServerConnectionDraft,
  plexAuthToken: string | null,
  effectiveFormLoginEnabled: boolean,
) {
  const supportsAuth =
    draft.provider === "JELLYFIN" || draft.provider === "PLEX" || draft.provider === "EMBY";
  const input: Record<string, unknown> = {
    provider: draft.provider,
    displayName: draft.displayName.trim(),
    baseUrl: draft.baseUrl.trim(),
    externalUrl: draft.externalUrl.trim(),
    enabled: draft.enabled,
    loginEnabled: supportsAuth && effectiveFormLoginEnabled && draft.loginEnabled,
    linkingEnabled: supportsAuth && draft.linkingEnabled,
    autoAddEnabled: supportsAuth && draft.autoAddEnabled,
    defaultAppPermissions: supportsAuth ? draft.defaultAppPermissions : [],
    defaultLibraryGrants: supportsAuth
      ? normalizeDefaultLibraryGrants(draft.defaultLibraryGrants)
      : [],
    pathMappings: parsePathMappings(draft.pathMappingsText),
  };

  const apiKey = normalizeOptional(draft.apiKey);
  const adminUsername = normalizeOptional(draft.adminUsername);
  const adminPassword = nonEmptySecret(draft.adminPassword);
  if (draft.provider === "PLEX" && draft.plexServerId && plexAuthToken) {
    input.plexServerId = draft.plexServerId;
    input.plexAuthToken = plexAuthToken;
  }
  if (draft.provider === "EMBY") {
    input.embyConnectionMode = draft.embyConnectionMode;
    input.embyConnectEnabled = draft.embyConnectEnabled;
    if (draft.embyConnectionMode === "LOCAL") {
      input.embyLocalSetupMethod = draft.embyLocalSetupMethod;
      if (draft.embyLocalSetupMethod === "API_KEY" && apiKey) input.apiKey = apiKey;
      if (draft.embyLocalSetupMethod === "ADMIN_CREDENTIALS") {
        if (adminUsername) input.adminUsername = adminUsername;
        if (adminPassword) input.adminPassword = adminPassword;
      }
    } else {
      const connectUsername = normalizeOptional(draft.embyConnectUsernameOrEmail);
      const connectPassword = nonEmptySecret(draft.embyConnectPassword);
      const connectServerId = normalizeOptional(draft.embyConnectServerId);
      if (connectUsername) input.embyConnectUsernameOrEmail = connectUsername;
      if (connectPassword) input.embyConnectPassword = connectPassword;
      if (connectServerId) input.embyConnectServerId = connectServerId;
    }
  }
  if (draft.provider === "JELLYFIN" && draft.jellyfinCredentialMode === "apiKey" && apiKey) {
    input.apiKey = apiKey;
  }
  if (draft.provider === "JELLYFIN" && draft.jellyfinCredentialMode === "adminLogin") {
    if (adminUsername) input.adminUsername = adminUsername;
    if (adminPassword) input.adminPassword = adminPassword;
  }
  return input;
}

function buildUpdateInput(
  id: string,
  draft: MediaServerConnectionDraft,
  plexAuthToken: string | null,
  effectiveFormLoginEnabled: boolean,
) {
  const input = buildCreateInput(draft, plexAuthToken, effectiveFormLoginEnabled);
  const apiKey = normalizeOptional(draft.apiKey);
  input.id = id;
  if (!effectiveFormLoginEnabled) delete input.loginEnabled;
  if (!apiKey) delete input.apiKey;
  if (draft.clearApiKey) input.clearApiKey = true;
  if (draft.provider === "EMBY") {
    const hasRotationCredentials =
      (draft.embyConnectionMode === "LOCAL" &&
        ((draft.embyLocalSetupMethod === "API_KEY" && Boolean(apiKey)) ||
          (draft.embyLocalSetupMethod === "ADMIN_CREDENTIALS" &&
            Boolean(normalizeOptional(draft.adminUsername)) &&
            Boolean(nonEmptySecret(draft.adminPassword))))) ||
      (draft.embyConnectionMode === "CONNECT" &&
        Boolean(normalizeOptional(draft.embyConnectUsernameOrEmail)) &&
        Boolean(nonEmptySecret(draft.embyConnectPassword)) &&
        Boolean(normalizeOptional(draft.embyConnectServerId)));
    if (!hasRotationCredentials) {
      delete input.embyConnectionMode;
      delete input.embyLocalSetupMethod;
      delete input.embyConnectUsernameOrEmail;
      delete input.embyConnectPassword;
      delete input.embyConnectServerId;
    }
  }
  return input;
}

export function SettingsMediaServersContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [connections, setConnections] =
    useState<SettingsMediaServersSectionProps["connections"]>([]);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [draft, setDraft] = useState<MediaServerConnectionDraft>(() =>
    cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT),
  );
  const [draftBaseline, setDraftBaseline] = useState<MediaServerConnectionDraft>(() =>
    cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT),
  );
  const [editingConnectionId, setEditingConnectionId] = useState<string | null>(null);
  const [mutatingConnectionId, setMutatingConnectionId] = useState<string | null>(null);
  const [testingConnectionId, setTestingConnectionId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [pendingDeleteConnection, setPendingDeleteConnection] =
    useState<MediaServerConnection | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingMediaServerEditorAction>(null);
  const [plexDiscoveryToken, setPlexDiscoveryToken] = useState<string | null>(null);
  const [plexServerOptions, setPlexServerOptions] = useState<PlexServerDiscovery[]>([]);
  const [plexDiscoveryBusy, setPlexDiscoveryBusy] = useState(false);
  const [embyConnectBusy, setEmbyConnectBusy] = useState(false);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [localPathStyle, setLocalPathStyle] =
    useState<LocalPathStyle | undefined>(undefined);
  const [pathMappingsValid, setPathMappingsValid] = useState(true);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState(false);

  const isDraftDirty = JSON.stringify(draft) !== JSON.stringify(draftBaseline);

  const refreshConnections = useCallback(async () => {
    const { data, error } = await client
      .query(mediaServerConnectionsQuery, { provider: null }, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setLocalPathStyle(
      localPathStyleFromRuntimeValue(data?.runtimeInfo?.runtimePathStyle),
    );
    setConnections(
      ((data?.mediaServerConnections ?? []) as MediaServerConnection[]).filter((connection) =>
        isVisibleMediaServerProvider(connection.provider),
      ),
    );
  }, [client]);

  const refreshLibraries = useCallback(async () => {
    const { data, error } = await client
      .query(librariesQuery, { facet: null, permission: "VIEW" }, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setLibraries((data?.libraries ?? []) as LibraryRecord[]);
  }, [client]);

  const refreshAuthRuntimeState = useCallback(async () => {
    const { data, error } = await client
      .query(authRuntimeStateQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setEffectiveFormLoginEnabled(data?.authRuntimeState?.effectiveFormLoginEnabled === true);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([refreshConnections(), refreshLibraries(), refreshAuthRuntimeState()])
      .catch((error: unknown) => {
        if (!cancelled) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [refreshConnections, refreshLibraries, refreshAuthRuntimeState, setGlobalStatus, t]);

  const resetDraft = useCallback(() => {
    setEditingConnectionId(null);
    const nextDraft = cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
    setPlexDiscoveryToken(null);
    setPlexServerOptions([]);
    setEditorError(null);
    setPathMappingsValid(true);
  }, []);

  const openCreateEditor = useCallback(() => {
    const nextDraft = cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT);
    setEditingConnectionId(null);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
    setPlexDiscoveryToken(null);
    setPlexServerOptions([]);
    setEditorError(null);
    setPathMappingsValid(true);
    setEditorMode("create");
    setIsEditorOpen(true);
  }, []);

  const openEditEditor = useCallback((connection: MediaServerConnection) => {
    const nextDraft = draftFromConnection(connection);
    setEditingConnectionId(connection.id);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
    setPlexDiscoveryToken(null);
    setPlexServerOptions([]);
    setEditorError(null);
    setPathMappingsValid(
      pathMappingsTextHasValidLocalPaths(nextDraft.pathMappingsText, localPathStyle),
    );
    setEditorMode("edit");
    setIsEditorOpen(true);
    setGlobalStatus(t("status.editingMediaServer", { name: connection.displayName }));
  }, [localPathStyle, setGlobalStatus, t]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditConnection = useCallback((connection: MediaServerConnection) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(connection);
      return;
    }
    setPendingEditorAction({ type: "edit", connection });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) {
      return;
    }
    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDraft();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, resetDraft]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) {
      return;
    }
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.connection);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDraft();
    }
    setPendingEditorAction(null);
  }, [openCreateEditor, openEditEditor, pendingEditorAction, resetDraft]);

  const discoverPlexServers = useCallback(async () => {
    setPlexDiscoveryBusy(true);
    try {
      const token = await authenticateWithPlexPin();
      const { data, error } = await client
        .mutation<{ discoverPlexMediaServers?: PlexServerDiscovery[] }>(
          discoverPlexMediaServersMutation,
          { plexAuthToken: token },
        )
        .toPromise();
      if (error) throw error;
      const servers = data?.discoverPlexMediaServers ?? [];
      setPlexDiscoveryToken(token);
      setPlexServerOptions(servers);
      setDraft((previous) => ({
        ...previous,
        plexServerId: servers.length === 1 ? servers[0].id : previous.plexServerId,
      }));
      setGlobalStatus(
        servers.length > 0
          ? t("status.plexServersDiscovered")
          : t("settings.plexServerDiscoveryEmpty"),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setPlexDiscoveryBusy(false);
    }
  }, [client, setGlobalStatus, t]);

  const discoverEmbyConnectServers = useCallback(async () => {
    setEmbyConnectBusy(true);
    try {
      const { data, error } = await client
        .mutation<{ discoverEmbyConnectServers?: EmbyConnectServer[] }>(
          discoverEmbyConnectServersMutation,
          {
            input: {
              usernameOrEmail: draft.embyConnectUsernameOrEmail,
              password: draft.embyConnectPassword,
            },
          },
        )
        .toPromise();
      if (error) throw error;
      const servers = data?.discoverEmbyConnectServers ?? [];
      setDraft((previous) => {
        const selected =
          servers.find((server) => server.serverId === previous.embyConnectServerId) ??
          (servers.length === 1 ? servers[0] : null);
        return {
          ...previous,
          embyDiscoveredServers: servers,
          embyConnectServerId: selected?.serverId ?? previous.embyConnectServerId,
          baseUrl: selected?.suggestedBaseUrl ?? previous.baseUrl,
        };
      });
      setGlobalStatus(
        servers.length > 0
          ? `Discovered ${servers.length} Emby Connect server${servers.length === 1 ? "" : "s"}`
          : "No Emby Connect servers were found",
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setEmbyConnectBusy(false);
    }
  }, [client, draft.embyConnectPassword, draft.embyConnectUsernameOrEmail, setGlobalStatus, t]);

  const testSavedEmbyConnect = useCallback(async () => {
    if (!editingConnectionId) return;
    setEmbyConnectBusy(true);
    try {
      const { data, error } = await client
        .mutation(testEmbyConnectMutation, {
          input: {
            connectionId: editingConnectionId,
            usernameOrEmail: draft.embyConnectUsernameOrEmail,
            password: draft.embyConnectPassword,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testEmbyConnect?.status !== "ok") {
        throw new Error(data?.testEmbyConnect?.message ?? "Emby Connect test failed");
      }
      setGlobalStatus("Emby Connect test passed");
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : "Emby Connect test failed");
    } finally {
      setEmbyConnectBusy(false);
    }
  }, [client, draft.embyConnectPassword, draft.embyConnectUsernameOrEmail, editingConnectionId, setGlobalStatus]);

  const submitConnection = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setEditorError(null);
    // Parse on save: `192.168.1.5:32400` or `plex.example.com` mean what they
    // plainly mean, and the form is put back with the scheme that was chosen so
    // nothing is decided behind the operator's back.
    const submitted = normalizeMediaServerConnectionDraft(draft);
    if (submitted !== draft) {
      setDraft(submitted);
    }
    const name = submitted.displayName.trim();
    const baseUrl = submitted.baseUrl.trim();
    if (!name || (submitted.provider !== "PLEX" && !baseUrl)) {
      const message = t("settings.mediaServerValidation");
      setEditorError(message);
      setGlobalStatus(message);
      return;
    }
    if (!pathMappingsTextHasValidLocalPaths(submitted.pathMappingsText, localPathStyle)) {
      const message = t("settings.downloadClientRemotePathMappingsLocalRequired");
      setPathMappingsValid(false);
      setEditorError(message);
      setGlobalStatus(message);
      return;
    }
    if (
      submitted.provider === "PLEX" &&
      (submitted.loginEnabled || submitted.linkingEnabled || submitted.autoAddEnabled) &&
      !submitted.machineIdPresent &&
      !submitted.plexServerId
    ) {
      const message = t("settings.plexServerDiscoveryRequired");
      setEditorError(message);
      setGlobalStatus(message);
      return;
    }

    setMutatingConnectionId(editingConnectionId ?? "new");
    try {
      if (editingConnectionId) {
        const { error } = await client.mutation(updateMediaServerConnectionMutation, {
          input: buildUpdateInput(
            editingConnectionId,
            submitted,
            plexDiscoveryToken,
            effectiveFormLoginEnabled,
          ),
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.mediaServerUpdated"));
      } else {
        const { error } = await client.mutation(createMediaServerConnectionMutation, {
          input: buildCreateInput(submitted, plexDiscoveryToken, effectiveFormLoginEnabled),
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.mediaServerCreated"));
      }

      resetDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      if (!isReportedConnectionFeedbackError(error)) {
        const message = error instanceof Error ? error.message : t("status.failedToUpdate");
        setEditorError(message);
        setGlobalStatus(message);
      }
    } finally {
      setMutatingConnectionId(null);
    }
  };

  const testConnection = useCallback(async (connection: MediaServerConnection) => {
    setTestingConnectionId(connection.id);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingMediaServerConnection", {
          server: connection.displayName,
        }),
        successMessage: t("status.mediaServerConnectionTestPassed", {
          server: connection.displayName,
        }),
        failureFallbackMessage: t("status.mediaServerConnectionTestFailed", {
          server: connection.displayName,
        }),
        run: async () => {
          const plexAuthToken =
            connection.provider === "PLEX" ? await authenticateWithPlexPin() : null;
          const { data, error } = await client
            .mutation(testMediaServerConnectionMutation, {
              input: {
                id: connection.id,
                plexAuthToken,
              },
            })
            .toPromise();
          if (error) throw error;
          const validation = data?.testMediaServerConnection;
          if (validation?.status !== "ok") {
            throw new Error(
              validation?.message ?? t("status.mediaServerConnectionTestFailed", {
                server: connection.displayName,
              }),
            );
          }
        },
      });
    } catch {
      // Shared connection feedback already surfaced the failure.
    } finally {
      setTestingConnectionId(null);
    }
  }, [client, setGlobalStatus, t]);

  const toggleConnectionEnabled = useCallback(async (connection: MediaServerConnection) => {
    const nextEnabled = !connection.enabled;
    setMutatingConnectionId(connection.id);
    try {
      const { error } = await client.mutation(updateMediaServerConnectionMutation, {
        input: {
          id: connection.id,
          enabled: nextEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.mediaServerUpdated"));
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingConnectionId(null);
    }
  }, [client, refreshConnections, setGlobalStatus, t]);

  const deleteConnection = useCallback((connection: MediaServerConnection) => {
    setPendingDeleteConnection(connection);
  }, []);

  const confirmDeleteConnection = useCallback(async () => {
    if (!pendingDeleteConnection) {
      return;
    }
    const connection = pendingDeleteConnection;
    setMutatingConnectionId(connection.id);
    try {
      const { error } = await client
        .mutation(deleteMediaServerConnectionMutation, { id: connection.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.mediaServerDeleted", { name: connection.displayName }));
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
      if (editingConnectionId === connection.id) {
        resetDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingConnectionId(null);
      setPendingDeleteConnection(null);
    }
  }, [
    client,
    editingConnectionId,
    pendingDeleteConnection,
    refreshConnections,
    resetDraft,
    setGlobalStatus,
    t,
  ]);

  return (
    <>
      <SettingsMediaServersSection
        connections={connections}
        libraries={libraries}
        draft={draft}
        setDraft={setDraft}
        localPathStyle={localPathStyle}
        pathMappingsValid={pathMappingsValid}
        onPathMappingsValidityChange={setPathMappingsValid}
        effectiveFormLoginEnabled={effectiveFormLoginEnabled}
        editingConnectionId={editingConnectionId}
        mutatingConnectionId={mutatingConnectionId}
        testingConnectionId={testingConnectionId}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        submitConnection={submitConnection}
        editConnection={requestEditConnection}
        testConnection={testConnection}
        toggleConnectionEnabled={toggleConnectionEnabled}
        deleteConnection={deleteConnection}
        resetDraft={requestCloseEditor}
        startCreateConnection={requestCreateEditor}
        plexServerOptions={plexServerOptions}
        plexDiscoveryBusy={plexDiscoveryBusy}
        discoverPlexServers={discoverPlexServers}
        embyConnectBusy={embyConnectBusy}
        discoverEmbyConnectServers={discoverEmbyConnectServers}
        testSavedEmbyConnect={testSavedEmbyConnect}
        editorError={editorError}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.mediaServerConfirmDiscardTitle")}
        description={t("settings.mediaServerConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.mediaServerCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingConnectionId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteConnection !== null}
        title={t("label.delete")}
        description={
          pendingDeleteConnection
            ? t("status.deletingMediaServer", { name: pendingDeleteConnection.displayName })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingConnectionId !== null}
        onConfirm={confirmDeleteConnection}
        onCancel={() => setPendingDeleteConnection(null)}
      />
    </>
  );
}
