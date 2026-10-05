import * as React from "react";
import { useClient } from "urql";
import { getRuntimeBasePath } from "@/lib/runtime-config";

import { useTranslate } from "@/lib/context/translate-context";
import { completeListAccountLinkMutation } from "@/lib/graphql/list-accounts";
import { listAccountReturn } from "@/lib/utils/list-account-return";
import { LIST_ACCOUNT_LINK_STORAGE_KEY, listAccountCompletionInput, listAccountReturnMatches } from "@/lib/utils/list-account-link";

export default function ListAccountReturnPage() {
  const t = useTranslate();
  const client = useClient();
  const started = React.useRef(false);
  const [status, setStatus] = React.useState("lists.accounts.finishing");
  React.useEffect(() => {
    if (started.current) return;
    started.current = true;
    const complete = async () => {
      const payload = listAccountReturn?.payload;
      const pending = listAccountReturn?.pending ?? null;
      if (!payload) { setStatus("lists.accounts.linkFailed"); return; }
      if (window.opener && !window.opener.closed) {
        window.opener.postMessage(payload, window.location.origin);
        try { window.sessionStorage.removeItem(LIST_ACCOUNT_LINK_STORAGE_KEY); } catch { /* The opener owns the pending session. */ }
        setStatus("lists.accounts.returnToLists");
        window.close();
        return;
      }
      try { window.sessionStorage.removeItem(LIST_ACCOUNT_LINK_STORAGE_KEY); } catch { /* URL material is already erased. */ }
      if (!listAccountReturnMatches(payload, pending) || !pending || payload.error) {
        setStatus("lists.accounts.linkExpired");
        return;
      }
      const result = await client.mutation(completeListAccountLinkMutation, listAccountCompletionInput(pending, payload)).toPromise();
      setStatus(result.error ? "lists.accounts.linkFailed" : "lists.accounts.linked");
    };
    void complete().catch(() => setStatus("lists.accounts.linkFailed"));
  }, [client]);
  return (
    <main className="mx-auto max-w-lg space-y-4 p-8">
      <h1 className="text-xl font-semibold">{t("lists.accounts.heading")}</h1>
      <p role="status">{t(status)}</p>
      <a className="underline" href={`${getRuntimeBasePath().replace(/\/$/, "")}/lists/personal`}>{t("lists.accounts.returnToLists")}</a>
    </main>
  );
}
