import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsProxiesSection } from "@/components/views/settings/settings-proxies-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { ProxyDraft, ProxyProviderTypeValue, ProxyRecord } from "@/lib/types";
import {
  PROXY_DEFAULT_BASE_URLS,
  PROXY_INITIAL_DRAFT,
  WIREGUARD_MTU_MAX,
  WIREGUARD_MTU_MIN,
  formatTunnelList,
  isSshTunnelProxyProvider,
  isWireguardProxyProvider,
  looksLikeWireguardKey,
  splitTunnelList,
  supportsProxyRemoteDns,
} from "@/lib/types";
import {
  buildCreateProxyInput,
  buildUpdateProxyInput,
} from "@/lib/utils/settings-mutation-inputs";
import { proxyConfigsQuery } from "@/lib/graphql/queries";
import {
  createProxyConfigMutation,
  deleteProxyConfigMutation,
  resetProxyHostKeyMutation,
  testProxyConfigMutation,
  updateProxyConfigMutation,
} from "@/lib/graphql/mutations";

export function SettingsProxiesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [proxyConfigs, setProxyConfigs] = useState<ProxyRecord[]>([]);
  const [editingProxyId, setEditingProxyId] = useState<string | null>(null);
  const [isProxyEditorOpen, setIsProxyEditorOpen] = useState(false);
  const [mutatingProxyId, setMutatingProxyId] = useState<string | null>(null);
  const [testingProxyId, setTestingProxyId] = useState<string | null>(null);
  const [resettingHostKeyProxyId, setResettingHostKeyProxyId] =
    useState<string | null>(null);
  const [pendingDeleteProxy, setPendingDeleteProxy] = useState<ProxyRecord | null>(
    null,
  );
  const [pendingHostKeyResetProxy, setPendingHostKeyResetProxy] =
    useState<ProxyRecord | null>(null);
  const [proxyDraft, setProxyDraft] = useState<ProxyDraft>(() => ({
    ...PROXY_INITIAL_DRAFT,
  }));

  const refreshProxyConfigs = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(proxyConfigsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setProxyConfigs(data?.proxyConfigs || []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshProxyConfigs();
  }, [refreshProxyConfigs]);

  const resetProxyDraft = useCallback(() => {
    setEditingProxyId(null);
    setIsProxyEditorOpen(false);
    setProxyDraft({ ...PROXY_INITIAL_DRAFT });
  }, []);

  const editProxy = useCallback(
    (proxy: ProxyRecord) => {
      setEditingProxyId(proxy.id);
      setIsProxyEditorOpen(true);
      setProxyDraft({
        ...PROXY_INITIAL_DRAFT,
        // Kept verbatim rather than coerced to a known value: the provider is
        // immutable while editing, and relabelling a row from a newer server
        // as something else would be a lie.
        providerType: proxy.providerType as ProxyProviderTypeValue,
        name: proxy.name,
        baseUrl: proxy.baseUrl,
        requestTimeoutSeconds: proxy.requestTimeoutSeconds,
        // Secrets are write-only: they are never read back, so the editor
        // opens blank, meaning "leave the stored secret alone".
        hasStoredCredentials: proxy.hasCredentials,
        hasStoredPrivateKey: proxy.hasPrivateKey,
        hasStoredPresharedKey: proxy.hasPresharedKey,
        // The peer's key and the two lists are not secrets, so they come back
        // in full and the editor shows what is actually stored.
        peerPublicKey: proxy.peerPublicKey ?? "",
        tunnelAddresses: formatTunnelList(proxy.tunnelAddresses),
        tunnelDnsServers: formatTunnelList(proxy.tunnelDnsServers),
        tunnelMtu: proxy.tunnelMtu === null ? "" : String(proxy.tunnelMtu),
        hasStoredTunnelMtu: proxy.tunnelMtu !== null,
        tunnelKeepaliveSeconds:
          proxy.tunnelKeepaliveSeconds === null
            ? ""
            : String(proxy.tunnelKeepaliveSeconds),
        hasStoredTunnelKeepaliveSeconds: proxy.tunnelKeepaliveSeconds !== null,
        remoteDns: proxy.remoteDns,
        isEnabled: proxy.isEnabled,
      });
      setGlobalStatus(t("status.editingProxy", { name: proxy.name }));
    },
    [setGlobalStatus, t],
  );

  const startCreateProxy = useCallback(() => {
    setEditingProxyId(null);
    setProxyDraft({ ...PROXY_INITIAL_DRAFT });
    setIsProxyEditorOpen(true);
  }, []);

  const changeProxyProvider = useCallback(
    (providerType: ProxyProviderTypeValue) => {
      setProxyDraft((prev) => {
        if (prev.providerType === providerType) {
          return prev;
        }
        const previousDefault = PROXY_DEFAULT_BASE_URLS[prev.providerType];
        return {
          ...prev,
          providerType,
          // Only reseed a URL the operator has not customized; a typed value
          // survives so switching provider by accident costs nothing.
          baseUrl:
            prev.baseUrl.trim() === "" || prev.baseUrl === previousDefault
              ? PROXY_DEFAULT_BASE_URLS[providerType]
              : prev.baseUrl,
          // Fields the new provider rejects must not linger in the draft.
          username: "",
          password: "",
          clearCredentials: false,
          clearPassword: false,
          privateKey: "",
          privateKeyPassphrase: "",
          clearPrivateKey: false,
          peerPublicKey: "",
          presharedKey: "",
          clearPresharedKey: false,
          tunnelAddresses: "",
          tunnelDnsServers: "",
          tunnelMtu: "",
          tunnelKeepaliveSeconds: "",
          remoteDns: supportsProxyRemoteDns(providerType) ? prev.remoteDns : false,
        };
      });
    },
    [],
  );

  /**
   * WireGuard's own rules, in the order the workflow checks them. The 44-char
   * base64 shape is a cheap pre-check used only to say something useful about
   * a key that is obviously wrong — the backend parses it and stays the
   * authority on whether it is usable.
   */
  const wireguardValidationMessage = useCallback(
    (draft: ProxyDraft): string | null => {
      const privateKey = draft.privateKey.trim();
      const keepsPrivateKey = draft.hasStoredPrivateKey && !draft.clearPrivateKey;
      if (privateKey === "" && !keepsPrivateKey) {
        return t("settings.proxyValidationWireguardPrivateKey");
      }
      if (privateKey !== "" && !looksLikeWireguardKey(privateKey)) {
        return t("settings.proxyValidationWireguardKeyShape", {
          field: t("settings.proxyPrivateKey"),
        });
      }
      const peerPublicKey = draft.peerPublicKey.trim();
      if (peerPublicKey === "") {
        return t("settings.proxyValidationWireguardPeerPublicKey");
      }
      if (!looksLikeWireguardKey(peerPublicKey)) {
        return t("settings.proxyValidationWireguardKeyShape", {
          field: t("settings.proxyPeerPublicKey"),
        });
      }
      const presharedKey = draft.presharedKey.trim();
      if (presharedKey !== "" && !looksLikeWireguardKey(presharedKey)) {
        return t("settings.proxyValidationWireguardKeyShape", {
          field: t("settings.proxyPresharedKey"),
        });
      }
      if (splitTunnelList(draft.tunnelAddresses).length === 0) {
        return t("settings.proxyValidationWireguardAddresses");
      }
      const mtu = draft.tunnelMtu.trim();
      if (mtu !== "") {
        const parsed = Number.parseInt(mtu, 10);
        if (
          Number.isNaN(parsed) ||
          parsed < WIREGUARD_MTU_MIN ||
          parsed > WIREGUARD_MTU_MAX
        ) {
          return t("settings.proxyValidationWireguardMtu", {
            min: WIREGUARD_MTU_MIN,
            max: WIREGUARD_MTU_MAX,
          });
        }
      }
      const keepalive = draft.tunnelKeepaliveSeconds.trim();
      if (keepalive !== "") {
        const parsed = Number.parseInt(keepalive, 10);
        if (Number.isNaN(parsed) || parsed < 0) {
          return t("settings.proxyValidationWireguardKeepalive");
        }
      }
      return null;
    },
    [t],
  );

  /**
   * The same rules the API enforces, so an obviously unusable tunnel is
   * refused before a round trip: an SSH tunnel needs a username and either a
   * password or a key; a WireGuard tunnel needs both halves of a key pair and
   * at least one address.
   */
  const tunnelValidationMessage = useCallback(
    (draft: ProxyDraft): string | null => {
      if (isSshTunnelProxyProvider(draft.providerType)) {
        const keepsUsername =
          draft.hasStoredCredentials && !draft.clearCredentials;
        if (draft.username.trim() === "" && !keepsUsername) {
          return t("settings.proxyValidationTunnelUsername");
        }
        const keepsPassword =
          draft.hasStoredCredentials &&
          !draft.clearCredentials &&
          !draft.clearPassword;
        const keepsPrivateKey =
          draft.hasStoredPrivateKey && !draft.clearPrivateKey;
        const hasAuth =
          draft.password.trim() !== "" ||
          draft.privateKey.trim() !== "" ||
          keepsPassword ||
          keepsPrivateKey;
        return hasAuth ? null : t("settings.proxyValidationTunnelAuth");
      }
      if (isWireguardProxyProvider(draft.providerType)) {
        return wireguardValidationMessage(draft);
      }
      return null;
    },
    [t, wireguardValidationMessage],
  );

  const submitProxy = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const name = proxyDraft.name.trim();
      const baseUrl = proxyDraft.baseUrl.trim();
      if (!name || !baseUrl) {
        setGlobalStatus(t("settings.proxyValidation"));
        return;
      }
      const tunnelProblem = tunnelValidationMessage(proxyDraft);
      if (tunnelProblem) {
        setGlobalStatus(tunnelProblem);
        return;
      }

      setMutatingProxyId(editingProxyId || "new");
      try {
        if (editingProxyId) {
          const { error } = await client
            .mutation(updateProxyConfigMutation, {
              input: buildUpdateProxyInput(editingProxyId, proxyDraft),
            })
            .toPromise();
          if (error) throw error;
          setGlobalStatus(t("status.proxyUpdated"));
        } else {
          const { error } = await client
            .mutation(createProxyConfigMutation, {
              input: buildCreateProxyInput(proxyDraft),
            })
            .toPromise();
          if (error) throw error;
          setGlobalStatus(t("status.proxyCreated"));
        }
        resetProxyDraft();
        await refreshProxyConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingProxyId(null);
      }
    },
    [
      client,
      editingProxyId,
      proxyDraft,
      refreshProxyConfigs,
      resetProxyDraft,
      setGlobalStatus,
      t,
      tunnelValidationMessage,
    ],
  );

  const testProxy = useCallback(
    async (proxy: ProxyRecord) => {
      setTestingProxyId(proxy.id);
      try {
        const { data, error } = await client
          .mutation(testProxyConfigMutation, { id: proxy.id })
          .toPromise();
        if (error) throw error;
        const result = data?.testProxyConfig;
        setGlobalStatus(
          result?.message ||
            (result?.ok ? t("status.proxyTestPassed") : t("status.proxyTestFailed")),
        );
        await refreshProxyConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.proxyTestFailed"),
        );
      } finally {
        setTestingProxyId(null);
      }
    },
    [client, refreshProxyConfigs, setGlobalStatus, t],
  );

  const deleteProxy = useCallback((proxy: ProxyRecord) => {
    setPendingDeleteProxy(proxy);
  }, []);

  /**
   * The tunnel's own public key is the one value the operator has to carry
   * back to their WireGuard server, so it gets a copy button. There is no
   * shared copy affordance in this app yet, so this is the same shape the API
   * keys panel uses, reported through the page's own status line.
   */
  const copyTunnelPublicKey = useCallback(
    async (publicKey: string) => {
      try {
        if (!navigator.clipboard) {
          throw new Error("clipboard unavailable");
        }
        await navigator.clipboard.writeText(publicKey);
        setGlobalStatus(t("status.proxyTunnelPublicKeyCopied"));
      } catch {
        setGlobalStatus(t("status.proxyTunnelPublicKeyCopyFailed"));
      }
    },
    [setGlobalStatus, t],
  );

  const confirmDeleteProxy = useCallback(async () => {
    if (!pendingDeleteProxy) {
      return;
    }
    const proxy = pendingDeleteProxy;
    setMutatingProxyId(proxy.id);
    try {
      const { error } = await client
        .mutation(deleteProxyConfigMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.proxyDeleted", { name: proxy.name }));
      if (editingProxyId === proxy.id) {
        resetProxyDraft();
      }
      await refreshProxyConfigs();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingProxyId(null);
      setPendingDeleteProxy(null);
    }
  }, [
    client,
    editingProxyId,
    pendingDeleteProxy,
    refreshProxyConfigs,
    resetProxyDraft,
    setGlobalStatus,
    t,
  ]);

  const confirmResetHostKey = useCallback(async () => {
    if (!pendingHostKeyResetProxy) {
      return;
    }
    const proxy = pendingHostKeyResetProxy;
    setResettingHostKeyProxyId(proxy.id);
    try {
      const { error } = await client
        .mutation(resetProxyHostKeyMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.proxyHostKeyReset", { name: proxy.name }));
      await refreshProxyConfigs();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setResettingHostKeyProxyId(null);
      setPendingHostKeyResetProxy(null);
    }
  }, [
    client,
    pendingHostKeyResetProxy,
    refreshProxyConfigs,
    setGlobalStatus,
    t,
  ]);

  return (
    <>
      <SettingsProxiesSection
        proxyConfigs={proxyConfigs}
        proxyDraft={proxyDraft}
        setProxyDraft={setProxyDraft}
        editingProxyId={editingProxyId}
        isProxyEditorOpen={isProxyEditorOpen}
        mutatingProxyId={mutatingProxyId}
        testingProxyId={testingProxyId}
        resettingHostKeyProxyId={resettingHostKeyProxyId}
        submitProxy={submitProxy}
        resetProxyDraft={resetProxyDraft}
        startCreateProxy={startCreateProxy}
        changeProxyProvider={changeProxyProvider}
        editProxy={editProxy}
        testProxy={testProxy}
        deleteProxy={deleteProxy}
        requestResetHostKey={setPendingHostKeyResetProxy}
        copyTunnelPublicKey={copyTunnelPublicKey}
      />
      <ConfirmDialog
        open={pendingDeleteProxy !== null}
        contentId="settings-indexer-proxy-delete-dialog"
        title={t("label.delete")}
        description={
          pendingDeleteProxy
            ? t("settings.proxyDeleteConfirmDescription", {
                name: pendingDeleteProxy.name,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-proxy-delete-confirm"
        cancelButtonId="settings-indexer-proxy-delete-cancel"
        isBusy={mutatingProxyId !== null}
        onConfirm={confirmDeleteProxy}
        onCancel={() => setPendingDeleteProxy(null)}
      />
      <ConfirmDialog
        open={pendingHostKeyResetProxy !== null}
        contentId="settings-indexer-proxy-host-key-reset-dialog"
        title={t("settings.proxyHostKeyReset")}
        description={t("settings.proxyHostKeyResetDescription")}
        confirmLabel={t("settings.proxyHostKeyReset")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-proxy-host-key-reset-confirm"
        cancelButtonId="settings-indexer-proxy-host-key-reset-cancel"
        isBusy={resettingHostKeyProxyId !== null}
        onConfirm={confirmResetHostKey}
        onCancel={() => setPendingHostKeyResetProxy(null)}
      />
    </>
  );
}
