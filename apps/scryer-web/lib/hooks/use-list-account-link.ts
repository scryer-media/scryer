import * as React from "react";
import { useClient } from "urql";

import { useTranslate } from "@/lib/context/translate-context";
import { hasGraphQlErrorCode } from "@/lib/graphql/error-message";
import { listErrorMessage } from "@/lib/utils/list-error-message";
import {
  cancelListAccountLinkMutation,
  completeListAccountLinkMutation,
  linkListAccountWithKeyMutation,
  pollListAccountLinkMutation,
  startListAccountLinkMutation,
} from "@/lib/graphql/list-accounts";
import type { ListAccountLinkSession } from "@/lib/types/lists";
import {
  LIST_ACCOUNT_LINK_STORAGE_KEY,
  LIST_ACCOUNT_POLL_INTERVAL_MS,
  finishCurrentListAccountLink,
  listAccountCompletionInput,
  listAccountPollRetryDelay,
  listAccountPollStep,
  parsePendingListAccountLink,
  validateListAccountMessage,
  type PendingListAccountLink,
} from "@/lib/utils/list-account-link";

type ActiveLink = {
  pending: PendingListAccountLink;
  popup: Window | null;
  consumed: boolean;
};

/** A key the member typed, and whether the provider still wants their approval after it. */
export type ListAccountMemberKey = { key: string; approval: boolean };

export function useListAccountLink(
  onLinked: (isCurrent: () => boolean) => Promise<void>,
) {
  const client = useClient();
  const t = useTranslate();
  const active = React.useRef<ActiveLink | null>(null);
  const generation = React.useRef(0);
  const [provider, setProvider] = React.useState<string | null>(null);
  // A key that links on its own opens no provider window to wait on.
  const [checkingKey, setCheckingKey] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const cancel = React.useCallback(() => {
    generation.current += 1;
    active.current?.popup?.close();
    active.current = null;
    try {
      window.sessionStorage.removeItem(LIST_ACCOUNT_LINK_STORAGE_KEY);
    } catch {
      /* Storage may be disabled. */
    }
    setProvider(null);
    setCheckingKey(false);
  }, []);

  // The member walked away from a link still in progress: the instance drops
  // its session too, so the authorization can no longer complete.
  const abandon = React.useCallback(() => {
    const link = active.current;
    if (link && !link.consumed) {
      void client
        .mutation(cancelListAccountLinkMutation, {
          sessionId: link.pending.sessionId,
        })
        .toPromise();
    }
    cancel();
  }, [cancel, client]);

  React.useEffect(() => {
    const receive = async (event: MessageEvent) => {
      const link = active.current;
      if (!link) return;
      const payload = validateListAccountMessage(
        event,
        link.pending,
        link.popup,
        window.location.origin,
        link.consumed,
      );
      if (!payload) return;
      const request = generation.current;
      link.consumed = true;
      try {
        if (payload.error) throw new Error(t("lists.accounts.linkFailed"));
        const result = await client
          .mutation(
            completeListAccountLinkMutation,
            listAccountCompletionInput(link.pending, payload),
          )
          .toPromise();
        if (request !== generation.current || active.current !== link) return;
        if (result.error) throw result.error;
        await finishCurrentListAccountLink(
          () => request === generation.current && active.current === link,
          onLinked,
          cancel,
          (reason) => {
            setError(
              listErrorMessage(reason, t, t("lists.accounts.linkFailed")),
            );
            cancel();
          },
        );
      } catch (reason) {
        if (request !== generation.current) return;
        setError(listErrorMessage(reason, t, t("lists.accounts.linkFailed")));
        cancel();
      }
    };
    const listener = (event: MessageEvent) => {
      void receive(event);
    };
    window.addEventListener("message", listener);
    return () => {
      window.removeEventListener("message", listener);
      const link = active.current;
      if (link && !link.consumed) {
        void client
          .mutation(cancelListAccountLinkMutation, {
            sessionId: link.pending.sessionId,
          })
          .toPromise();
      }
      generation.current += 1;
      active.current?.popup?.close();
      active.current = null;
    };
  }, [cancel, client, onLinked, t]);

  const start = React.useCallback(
    async (nextProvider: string, memberKey?: ListAccountMemberKey) => {
      abandon();
      setError(null);
      setProvider(nextProvider);
      const keyOnly = !!memberKey && !memberKey.approval;
      setCheckingKey(keyOnly);
      const request = generation.current;
      // Open during the click, before the network request loses user activation.
      const popup = keyOnly
        ? null
        : window.open("about:blank", "_blank", "popup,width=600,height=750");
      try {
        let session: ListAccountLinkSession | undefined;
        if (memberKey) {
          const result = await client
            .mutation(linkListAccountWithKeyMutation, {
              provider: nextProvider,
              key: memberKey.key,
              origin: memberKey.approval ? window.location.origin : null,
            })
            .toPromise();
          if (request !== generation.current) {
            popup?.close();
            return;
          }
          if (result.error) throw result.error;
          const linked = result.data?.linkListAccountWithKey as
            | { account: unknown; link: ListAccountLinkSession | null }
            | undefined;
          if (linked?.account) {
            popup?.close();
            await finishCurrentListAccountLink(
              () => request === generation.current,
              onLinked,
              cancel,
              (reason) => {
                setError(
                  listErrorMessage(reason, t, t("lists.accounts.linkFailed")),
                );
                cancel();
              },
            );
            return;
          }
          session = linked?.link ?? undefined;
        } else {
          const result = await client
            .mutation(startListAccountLinkMutation, {
              provider: nextProvider,
              origin: window.location.origin,
            })
            .toPromise();
          if (request !== generation.current) {
            popup?.close();
            return;
          }
          if (result.error) throw result.error;
          session = result.data?.startListAccountLink as
            ListAccountLinkSession | undefined;
        }
        if (!session) throw new Error(t("lists.accounts.linkFailed"));
        const pending = parsePendingListAccountLink(
          JSON.stringify({ ...session, provider: nextProvider }),
          window.location.origin,
        );
        if (!pending) throw new Error(t("lists.accounts.linkFailed"));
        const url = new URL(session.authorizeUrl);
        if (url.protocol !== "https:" || url.username || url.password)
          throw new Error(t("lists.accounts.linkFailed"));
        window.sessionStorage.setItem(
          LIST_ACCOUNT_LINK_STORAGE_KEY,
          JSON.stringify(pending),
        );
        if (!popup) {
          if (session.pollRequired)
            throw new Error(t("lists.accounts.popupBlocked"));
          window.location.assign(url.href);
          return;
        }
        popup.sessionStorage.setItem(
          LIST_ACCOUNT_LINK_STORAGE_KEY,
          JSON.stringify(pending),
        );
        active.current = { pending, popup, consumed: false };
        popup.location.replace(url.href);

        const poll = async () => {
          if (request !== generation.current || active.current?.consumed)
            return;
          let delayMs = LIST_ACCOUNT_POLL_INTERVAL_MS;
          if (Date.now() >= Date.parse(pending.expiresAt)) {
            setError(t("lists.accounts.linkExpired"));
            cancel();
            return;
          }
          if (session.pollRequired) {
            try {
              const polled = await client
                .mutation(pollListAccountLinkMutation, {
                  sessionId: pending.sessionId,
                })
                .toPromise();
              if (request !== generation.current) return;
              if (polled.error) throw polled.error;
              if (polled.data?.pollListAccountLink?.account) {
                if (active.current) active.current.consumed = true;
                await finishCurrentListAccountLink(
                  () => request === generation.current,
                  onLinked,
                  cancel,
                  (reason) => {
                    setError(
                      listErrorMessage(
                        reason,
                        t,
                        t("lists.accounts.linkFailed"),
                      ),
                    );
                    cancel();
                  },
                );
                return;
              }
              const step = listAccountPollStep(
                polled.data?.pollListAccountLink?.status,
              );
              if (step.kind === "failed") {
                setError(t("lists.accounts.linkFailed"));
                cancel();
                const refreshed = generation.current;
                void onLinked(() => refreshed === generation.current).catch(
                  () => undefined,
                );
                return;
              }
              delayMs = step.delayMs;
              setError(step.retrying ? t("lists.accounts.pollRetrying") : null);
            } catch (reason) {
              if (request !== generation.current) return;
              const retryDelay = listAccountPollRetryDelay(reason);
              if (retryDelay === null) {
                setError(
                  listErrorMessage(reason, t, t("lists.accounts.linkFailed")),
                );
                cancel();
                // An earlier poll may have linked the account before this one
                // failed; refresh once so it shows either way.
                const refreshed = generation.current;
                void onLinked(() => refreshed === generation.current).catch(
                  () => undefined,
                );
                return;
              }
              // A dropped request or a passing instance error does not end the
              // session; the instance keeps it, and answers a repeated poll even
              // if the first one linked it.
              delayMs = retryDelay;
              setError(t("lists.accounts.pollRetrying"));
            }
          } else if (popup.closed) {
            cancel();
            return;
          }
          if (request === generation.current)
            window.setTimeout(() => {
              void poll();
            }, delayMs);
        };
        void poll();
      } catch (reason) {
        popup?.close();
        if (request !== generation.current) return;
        setError(
          hasGraphQlErrorCode(reason, "LIST_ACCOUNT_ORIGIN_NOT_ALLOWED")
            ? t("lists.accounts.originNotAllowed")
            : listErrorMessage(reason, t, t("lists.accounts.linkFailed")),
        );
        cancel();
      }
    },
    [abandon, cancel, client, onLinked, t],
  );

  return { provider, checkingKey, error, start, cancel: abandon };
}
