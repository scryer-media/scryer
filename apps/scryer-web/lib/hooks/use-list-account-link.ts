import * as React from "react";
import { useClient } from "urql";

import { useTranslate } from "@/lib/context/translate-context";
import { hasGraphQlErrorCode, userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { completeListAccountLinkMutation, pollListAccountLinkMutation, startListAccountLinkMutation } from "@/lib/graphql/list-accounts";
import type { ListAccountLinkSession } from "@/lib/types/lists";
import {
  LIST_ACCOUNT_LINK_STORAGE_KEY,
  finishCurrentListAccountLink,
  listAccountCompletionInput,
  listAccountPollFailureIsFinal,
  parsePendingListAccountLink,
  validateListAccountMessage,
  type PendingListAccountLink,
} from "@/lib/utils/list-account-link";

type ActiveLink = { pending: PendingListAccountLink; popup: Window | null; consumed: boolean };

export function useListAccountLink(onLinked: (isCurrent: () => boolean) => Promise<void>) {
  const client = useClient();
  const t = useTranslate();
  const active = React.useRef<ActiveLink | null>(null);
  const generation = React.useRef(0);
  const [provider, setProvider] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const cancel = React.useCallback(() => {
    generation.current += 1;
    active.current?.popup?.close();
    active.current = null;
    try { window.sessionStorage.removeItem(LIST_ACCOUNT_LINK_STORAGE_KEY); } catch { /* Storage may be disabled. */ }
    setProvider(null);
  }, []);

  React.useEffect(() => {
    const receive = async (event: MessageEvent) => {
      const link = active.current;
      if (!link) return;
      const payload = validateListAccountMessage(event, link.pending, link.popup, window.location.origin, link.consumed);
      if (!payload) return;
      const request = generation.current;
      link.consumed = true;
      try {
        if (payload.error) throw new Error(t("lists.accounts.linkFailed"));
        const result = await client.mutation(completeListAccountLinkMutation, listAccountCompletionInput(link.pending, payload)).toPromise();
        if (request !== generation.current || active.current !== link) return;
        if (result.error) throw result.error;
        await finishCurrentListAccountLink(
          () => request === generation.current && active.current === link,
          onLinked,
          cancel,
          (reason) => {
            setError(userFacingGraphQlErrorMessage(reason, t("lists.accounts.linkFailed")));
            cancel();
          },
        );
      } catch (reason) {
        if (request !== generation.current) return;
        setError(userFacingGraphQlErrorMessage(reason, t("lists.accounts.linkFailed")));
        cancel();
      }
    };
    const listener = (event: MessageEvent) => { void receive(event); };
    window.addEventListener("message", listener);
    return () => {
      window.removeEventListener("message", listener);
      generation.current += 1;
      active.current?.popup?.close();
      active.current = null;
    };
  }, [cancel, client, onLinked, t]);

  const start = React.useCallback(async (nextProvider: string) => {
    cancel();
    setError(null);
    setProvider(nextProvider);
    const request = generation.current;
    // Open during the click, before the network request loses user activation.
    const popup = window.open("about:blank", "_blank", "popup,width=600,height=750");
    try {
      const result = await client.mutation(startListAccountLinkMutation, {
        provider: nextProvider,
        origin: window.location.origin,
      }).toPromise();
      if (request !== generation.current) { popup?.close(); return; }
      if (result.error) throw result.error;
      const session = result.data?.startListAccountLink as ListAccountLinkSession | undefined;
      if (!session) throw new Error(t("lists.accounts.linkFailed"));
      const pending = parsePendingListAccountLink(JSON.stringify({ ...session, provider: nextProvider }), window.location.origin);
      if (!pending) throw new Error(t("lists.accounts.linkFailed"));
      const url = new URL(session.authorizeUrl);
      if (url.protocol !== "https:" || url.username || url.password) throw new Error(t("lists.accounts.linkFailed"));
      window.sessionStorage.setItem(LIST_ACCOUNT_LINK_STORAGE_KEY, JSON.stringify(pending));
      if (!popup) {
        if (session.pollRequired) throw new Error(t("lists.accounts.popupBlocked"));
        window.location.assign(url.href);
        return;
      }
      popup.sessionStorage.setItem(LIST_ACCOUNT_LINK_STORAGE_KEY, JSON.stringify(pending));
      active.current = { pending, popup, consumed: false };
      popup.location.replace(url.href);

      const poll = async () => {
        if (request !== generation.current || active.current?.consumed) return;
        if (Date.now() >= Date.parse(pending.expiresAt)) {
          setError(t("lists.accounts.linkExpired"));
          cancel();
          return;
        }
        if (session.pollRequired) {
          try {
            const polled = await client.mutation(pollListAccountLinkMutation, { sessionId: pending.sessionId }).toPromise();
            if (request !== generation.current) return;
            if (polled.error) throw polled.error;
            if (polled.data?.pollListAccountLink?.account) {
              if (active.current) active.current.consumed = true;
              await finishCurrentListAccountLink(
                () => request === generation.current,
                onLinked,
                cancel,
                (reason) => {
                  setError(userFacingGraphQlErrorMessage(reason, t("lists.accounts.linkFailed")));
                  cancel();
                },
              );
              return;
            }
            if (["FAILED", "EXPIRED"].includes(polled.data?.pollListAccountLink?.status)) {
              setError(t("lists.accounts.linkFailed"));
              cancel();
              return;
            }
            setError(null);
          } catch (reason) {
            if (request !== generation.current) return;
            if (listAccountPollFailureIsFinal(reason)) {
              setError(userFacingGraphQlErrorMessage(reason, t("lists.accounts.linkFailed")));
              cancel();
              return;
            }
            // A dropped request or a provider hiccup does not end the session;
            // keep checking at the normal pace until it expires or is cancelled.
            setError(t("lists.accounts.pollRetrying"));
          }
        } else if (popup.closed) {
          cancel();
          return;
        }
        if (request === generation.current) window.setTimeout(() => { void poll(); }, 1500);
      };
      void poll();
    } catch (reason) {
      popup?.close();
      if (request !== generation.current) return;
      setError(hasGraphQlErrorCode(reason, "LIST_ACCOUNT_ORIGIN_NOT_ALLOWED")
        ? t("lists.accounts.originNotAllowed")
        : userFacingGraphQlErrorMessage(reason, t("lists.accounts.linkFailed")));
      cancel();
    }
  }, [cancel, client, onLinked, t]);

  return { provider, error, start, cancel };
}
