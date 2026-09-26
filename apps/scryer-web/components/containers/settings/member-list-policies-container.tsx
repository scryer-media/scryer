import * as React from "react";
import { useClient } from "urql";

import { MemberListPoliciesPanel } from "@/components/views/settings/member-list-policies-panel";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { setMemberListPolicyMutation } from "@/lib/graphql/mutations";
import { listMemberPoliciesQuery } from "@/lib/graphql/queries";
import type { ListPolicy, MemberListPolicy } from "@/lib/types/lists";

export function MemberListPoliciesContainer() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [policies, setPolicies] = React.useState<MemberListPolicy[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [savingUserId, setSavingUserId] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    try {
      const result = await client
        .query(listMemberPoliciesQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error) throw result.error;
      setPolicies((result.data?.listMemberPolicies ?? []) as MemberListPolicy[]);
      setError(null);
    } catch (loadError) {
      setError(userFacingGraphQlErrorMessage(loadError, t("status.failedToLoad")));
    } finally {
      setLoading(false);
    }
  }, [client, t]);

  React.useEffect(() => {
    void load();
  }, [load]);

  const change = React.useCallback(
    async (userId: string, policy: ListPolicy) => {
      const previous = policies;
      setSavingUserId(userId);
      setPolicies((current) => current.map((entry) => (entry.user.id === userId ? { ...entry, policy } : entry)));
      try {
        const result = await client.mutation(setMemberListPolicyMutation, { userId, policy }).toPromise();
        if (result.error) throw result.error;
        const saved = result.data?.setMemberListPolicy as MemberListPolicy | undefined;
        if (saved) {
          setPolicies((current) => current.map((entry) => (entry.user.id === userId ? saved : entry)));
        }
      } catch (saveError) {
        setPolicies(previous);
        setGlobalStatus(userFacingGraphQlErrorMessage(saveError, t("status.failedToUpdate")));
      } finally {
        setSavingUserId(null);
      }
    },
    [client, policies, setGlobalStatus, t],
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
