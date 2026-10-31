import * as React from "react";
import { useClient } from "urql";

import { MemberListPoliciesPanel } from "@/components/views/settings/member-list-policies-panel";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { listErrorMessage } from "@/lib/utils/list-error-message";
import { setMemberListPolicyMutation } from "@/lib/graphql/mutations";
import { listMemberPoliciesQuery } from "@/lib/graphql/queries";
import { useListsReactiveRefresh } from "@/lib/hooks/use-lists-reactive-refresh";
import type { ListPolicy, MemberListPolicy } from "@/lib/types/lists";
import {
  latestRead,
  memberListPoliciesChanged,
  rollBackMemberListPolicy,
} from "@/lib/utils/lists";

export function MemberListPoliciesContainer() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [policies, setPolicies] = React.useState<MemberListPolicy[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [savingUserId, setSavingUserId] = React.useState<string | null>(null);

  // Only the newest read may land: a refresh started by a live event can race
  // the first read, an earlier refresh, or a change made here.
  const [reads] = React.useState(latestRead);

  // A background read leaves the table on screen, and what it shows if it fails.
  const load = React.useCallback(
    async ({ background = false }: { background?: boolean } = {}) => {
      const isNewest = reads.begin();
      if (!background) setLoading(true);
      try {
        const result = await client
          .query(listMemberPoliciesQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (!isNewest()) return;
        if (result.error) throw result.error;
        setPolicies(
          (result.data?.listMemberPolicies ?? []) as MemberListPolicy[],
        );
        setError(null);
      } catch (loadError) {
        if (!background)
          setError(listErrorMessage(loadError, t, t("status.failedToLoad")));
      } finally {
        if (!background) setLoading(false);
      }
    },
    [client, reads, t],
  );

  React.useEffect(() => {
    void load();
  }, [load]);

  // A policy changed in another session is read again here.
  useListsReactiveRefresh(memberListPoliciesChanged, () => {
    void load({ background: true });
  });

  const change = React.useCallback(
    async (userId: string, policy: ListPolicy) => {
      const previous = policies.find((entry) => entry.user.id === userId);
      setSavingUserId(userId);
      // A read still in flight predates this change and must not undo it.
      reads.invalidate();
      setPolicies((current) =>
        current.map((entry) =>
          entry.user.id === userId ? { ...entry, policy } : entry,
        ),
      );
      try {
        const result = await client
          .mutation(setMemberListPolicyMutation, { userId, policy })
          .toPromise();
        if (result.error) throw result.error;
        const saved = result.data?.setMemberListPolicy as
          MemberListPolicy | undefined;
        if (saved) {
          reads.invalidate();
          setPolicies((current) =>
            current.map((entry) => (entry.user.id === userId ? saved : entry)),
          );
        }
      } catch (saveError) {
        if (previous) {
          setPolicies((current) =>
            rollBackMemberListPolicy(current, previous, policy),
          );
        }
        setGlobalStatus(
          listErrorMessage(saveError, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
      } finally {
        setSavingUserId(null);
      }
    },
    [client, policies, reads, setGlobalStatus, t],
  );

  return (
    <MemberListPoliciesPanel
      policies={policies}
      loading={loading}
      error={error}
      savingUserId={savingUserId}
      onChange={(userId, policy) => void change(userId, policy)}
    />
  );
}
