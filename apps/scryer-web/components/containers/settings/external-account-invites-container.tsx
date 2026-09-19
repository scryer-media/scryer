import type * as React from "react";
import { useCallback, useEffect, useState } from "react";
import {
  ExternalAccountInvitesPanel,
  type ExternalInviteDraft,
  type ExternalInviteMediaServerUserGroup,
  type ExternalInviteUser,
} from "@/components/views/settings/external-account-invites-panel";
import {
  createExternalAccountInviteMutation,
  unlinkExternalAccountMutation,
} from "@/lib/graphql/mutations";
import {
  externalAuthRuntimeSettingsQuery,
  externalAccountInvitesQuery,
  mediaServerUsersQuery,
  usersQuery,
} from "@/lib/graphql/queries";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useClient } from "urql";
import type {
  ExternalAuthRuntimeSettings,
  LinkedAccount,
} from "@/lib/types/settings";

const DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS: ExternalAuthRuntimeSettings = {
  loginProviders: [],
  linkingProviders: [],
  connections: [],
};

const DEFAULT_EXTERNAL_INVITE_DRAFT: ExternalInviteDraft = {
  userId: "",
  provider: "JELLYFIN",
  connectionId: "",
  providerUserIdentifier: "",
  providerUserId: "",
};

const EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT =
  "scryer:external-account-invite-sources-changed";

export function notifyExternalAccountInviteSourcesChanged() {
  if (typeof window !== "undefined") {
    window.dispatchEvent(
      new Event(EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT),
    );
  }
}

function visibleExternalAccountInvites(
  invites: LinkedAccount[],
): LinkedAccount[] {
  return invites.filter((invite) =>
    isVisibleExternalAccountProvider(invite.provider),
  );
}

type ExternalAccountInvitesContainerProps = {
  showMediaServersLink?: boolean;
};

export function ExternalAccountInvitesContainer({
  showMediaServersLink = false,
}: ExternalAccountInvitesContainerProps = {}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [users, setUsers] = useState<ExternalInviteUser[]>([]);
  const [invites, setInvites] = useState<LinkedAccount[]>([]);
  const [mediaServerUserGroups, setMediaServerUserGroups] = useState<
    ExternalInviteMediaServerUserGroup[]
  >([]);
  const [mediaServerUserSearchLoading, setMediaServerUserSearchLoading] =
    useState(false);
  const [mediaServerUserLookupError, setMediaServerUserLookupError] = useState<
    string | null
  >(null);
  const [externalAuthSettings, setExternalAuthSettings] =
    useState<ExternalAuthRuntimeSettings>(
      DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS,
    );
  const [externalInviteDraft, setExternalInviteDraft] =
    useState<ExternalInviteDraft>(DEFAULT_EXTERNAL_INVITE_DRAFT);
  const [loading, setLoading] = useState(true);
  const [externalInviteSubmitting, setExternalInviteSubmitting] =
    useState(false);
  const [unlinkingAccountId, setUnlinkingAccountId] = useState<string | null>(null);

  const updateExternalInviteDraft = useCallback(
    (patch: Partial<ExternalInviteDraft>) => {
      setMediaServerUserLookupError(null);
      setExternalInviteDraft((previous) => ({ ...previous, ...patch }));
    },
    [],
  );

  const refreshExternalInvites = useCallback(async () => {
    const { data, error } = await client
      .query(externalAccountInvitesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setInvites(
      visibleExternalAccountInvites(
        (data?.externalAccountInvites ?? []) as LinkedAccount[],
      ),
    );
  }, [client]);

  const refreshInviteData = useCallback(async () => {
    setLoading(true);
    try {
      const [usersResult, externalAuthResult, invitesResult] =
        await Promise.all([
          client
            .query(usersQuery, {}, { requestPolicy: "network-only" })
            .toPromise(),
          client
            .query<{
              externalAuthRuntimeSettings?: ExternalAuthRuntimeSettings;
            }>(
              externalAuthRuntimeSettingsQuery,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise(),
          client
            .query(
              externalAccountInvitesQuery,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise(),
        ]);
      if (usersResult.error) throw usersResult.error;
      if (externalAuthResult.error) throw externalAuthResult.error;
      if (invitesResult.error) throw invitesResult.error;

      setUsers(
        ((usersResult.data?.users ?? []) as ExternalInviteUser[]).map(
          (user) => ({
            id: user.id,
            username: user.username,
          }),
        ),
      );
      setExternalAuthSettings(
        externalAuthResult.data?.externalAuthRuntimeSettings ??
          DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS,
      );
      setInvites(
        visibleExternalAccountInvites(
          (invitesResult.data?.externalAccountInvites ?? []) as LinkedAccount[],
        ),
      );
    } catch (error) {
      setExternalAuthSettings(DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS);
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshInviteData();
  }, [refreshInviteData]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const handleInviteSourcesChanged = () => {
      void refreshInviteData();
    };

    window.addEventListener(
      EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT,
      handleInviteSourcesChanged,
    );
    return () => {
      window.removeEventListener(
        EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT,
        handleInviteSourcesChanged,
      );
    };
  }, [refreshInviteData]);

  useEffect(() => {
    setExternalInviteDraft((previous) => {
      return {
        ...previous,
        userId: users.some((user) => user.id === previous.userId)
          ? previous.userId
          : (users[0]?.id ?? ""),
      };
    });
  }, [users]);

  useEffect(() => {
    let cancelled = false;
    const search = externalInviteDraft.providerUserIdentifier.trim();
    setMediaServerUserSearchLoading(true);
    setMediaServerUserLookupError(null);

    const timeoutId = window.setTimeout(
      () => {
        void client
          .query(
            mediaServerUsersQuery,
            {
              search: search.length > 0 ? search : null,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise()
          .then(({ data, error }) => {
            if (cancelled) return;
            if (error) {
              setMediaServerUserGroups([]);
              setMediaServerUserLookupError(error.message);
              return;
            }
            setMediaServerUserGroups(
              (data?.mediaServerUsers ??
                []) as ExternalInviteMediaServerUserGroup[],
            );
          })
          .catch((error: unknown) => {
            if (cancelled) return;
            setMediaServerUserGroups([]);
            setMediaServerUserLookupError(
              error instanceof Error
                ? error.message
                : t("settings.mediaServerUserSearchFailed"),
            );
          })
          .finally(() => {
            if (!cancelled) {
              setMediaServerUserSearchLoading(false);
            }
          });
      },
      search.length > 0 ? 250 : 0,
    );

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [client, externalInviteDraft.providerUserIdentifier, t]);

  const createExternalAccountInvite = async (
    event: React.FormEvent<HTMLFormElement>,
  ) => {
    event.preventDefault();
    const userId = externalInviteDraft.userId.trim();
    const connectionId = externalInviteDraft.connectionId.trim();
    const providerUserIdentifier =
      externalInviteDraft.providerUserIdentifier.trim();
    const providerUserId = externalInviteDraft.providerUserId.trim();

    if (!userId || !connectionId || !providerUserId) {
      setGlobalStatus(t("settings.externalAccountInviteRequired"));
      return;
    }

    setExternalInviteSubmitting(true);
    try {
      const { error } = await client
        .mutation(createExternalAccountInviteMutation, {
          input: {
            userId,
            provider: externalInviteDraft.provider,
            connectionId,
            providerUserIdentifier,
            providerUserId,
          },
        })
        .toPromise();
      if (error) throw error;
      setExternalInviteDraft((previous) => ({
        ...previous,
        providerUserIdentifier: "",
        providerUserId: "",
      }));
      setGlobalStatus(t("settings.externalAccountInviteCreated"));
      await refreshExternalInvites();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error
          ? error.message
          : t("settings.externalAccountInviteFailed"),
      );
    } finally {
      setExternalInviteSubmitting(false);
    }
  };

  const unlinkExternalAccount = async (id: string) => {
    setUnlinkingAccountId(id);
    try {
      const { data, error } = await client
        .mutation<{ unlinkExternalAccount?: { linkedAccountId: string } }>(
          unlinkExternalAccountMutation,
          { linkedAccountId: id },
        )
        .toPromise();
      if (error) throw error;
      if (data?.unlinkExternalAccount?.linkedAccountId !== id) {
        throw new Error(t("profile.linkedAccountUnlinkFailed"));
      }
      setInvites((current) => current.filter((account) => account.id !== id));
      setGlobalStatus(t("profile.linkedAccountUnlinked"));
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("profile.linkedAccountUnlinkFailed"),
      );
    } finally {
      setUnlinkingAccountId(null);
    }
  };

  return (
    <ExternalAccountInvitesPanel
      users={users}
      invites={invites}
      mediaServerUserGroups={mediaServerUserGroups}
      mediaServerUserSearchLoading={mediaServerUserSearchLoading}
      mediaServerUserLookupError={mediaServerUserLookupError}
      externalAuthSettings={externalAuthSettings}
      loading={loading}
      externalInviteDraft={externalInviteDraft}
      externalInviteSubmitting={externalInviteSubmitting}
      updateExternalInviteDraft={updateExternalInviteDraft}
      createExternalAccountInvite={createExternalAccountInvite}
      unlinkExternalAccount={unlinkExternalAccount}
      unlinkingAccountId={unlinkingAccountId}
      showMediaServersLink={showMediaServersLink}
    />
  );
}
