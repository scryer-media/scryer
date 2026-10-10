import * as React from "react";
import { useClient, type Client } from "urql";

import { titleTagDefinitionsQuery } from "@/lib/graphql/queries";
import type { TitleTagDefinition } from "@/lib/types/title-tags";

type TitleTagDefinitionPayload = {
  id: string;
  label: string;
  description?: string | null;
  titleCount: number;
  seriesMovieCount?: number | null;
  createdAt: string;
};

type RegistryListener = (payload?: TitleTagDefinitionPayload[]) => void;
const registryListeners = new WeakMap<Client, Set<RegistryListener>>();
const registryRequests = new WeakMap<Client, Promise<TitleTagDefinitionPayload[]>>();

async function fetchRegistry(client: Client, requestPolicy: "cache-first" | "network-only") {
  const existing = registryRequests.get(client);
  if (existing) return existing;
  const pending = client
    .query(titleTagDefinitionsQuery, {}, { requestPolicy })
    .toPromise()
    .then((result) => {
      if (result.error) throw result.error;
      return (result.data?.titleTagDefinitions ?? []) as TitleTagDefinitionPayload[];
    });
  registryRequests.set(client, pending);
  try {
    return await pending;
  } finally {
    if (registryRequests.get(client) === pending) registryRequests.delete(client);
  }
}
export function refreshTitleTagConsumers(client: Client, payload?: TitleTagDefinitionPayload[]) {
  // A read started before creation cannot satisfy the post-creation refresh.
  registryRequests.delete(client);
  registryListeners.get(client)?.forEach((refresh) => refresh(payload));
}

export function fromTitleTagDefinitionPayload(
  payload: TitleTagDefinitionPayload,
): TitleTagDefinition {
  return {
    id: payload.id,
    label: payload.label,
    description: payload.description ?? null,
    titleCount: payload.titleCount,
    seriesMovieCount: payload.seriesMovieCount ?? 0,
    createdAt: payload.createdAt,
  };
}

/**
 * The administrator-defined tag vocabulary, read once per mounting component.
 *
 * Every tag surface — the per-title picker, the bulk dialog, the catalog
 * filter — is registry-backed and offers no free text, so each of them needs
 * this list before it can render a control at all. Registry reads are open to
 * any authenticated caller, so no permission gate sits in front of it.
 *
 * `enabled` exists for surfaces that are mounted long before they are shown:
 * the bulk-edit dialog lives in the media page's tree the whole time, so an
 * unconditional read would fetch the vocabulary on every page load for a dialog
 * most sessions never open. A disabled hook fetches nothing and reports
 * `loading: false`, which renders as an empty registry until it is enabled.
 */
export function useTitleTagDefinitions(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true;
  const client = useClient();
  const [definitions, setDefinitions] = React.useState<TitleTagDefinition[]>([]);
  const [loading, setLoading] = React.useState(enabled);
  const [error, setError] = React.useState(false);
  const loadSequence = React.useRef(0);

  // The vocabulary is small, changes rarely, and is read by three surfaces at
  // once, so the first read goes through urql's document cache. An explicit
  // reload skips it: `deleteTitleTagDefinition` returns a payload type the
  // query never selects, so urql cannot invalidate the list on its own.
  const load = React.useCallback(
    async (requestPolicy: "cache-first" | "network-only") => {
      const sequence = ++loadSequence.current;
      setLoading(true);
      try {
        const payload = await fetchRegistry(client, requestPolicy);
        if (sequence !== loadSequence.current) return;
        setDefinitions(payload.map(fromTitleTagDefinitionPayload));
        setError(false);
      } catch {
        if (sequence !== loadSequence.current) return;
        setDefinitions([]);
        setError(true);
      } finally {
        if (sequence === loadSequence.current) setLoading(false);
      }
    },
    [client],
  );

  const reload = React.useCallback(() => load("network-only"), [load]);

  React.useEffect(() => {
    if (!enabled) return;
    const listeners = registryListeners.get(client) ?? new Set<RegistryListener>();
    registryListeners.set(client, listeners);
    const refresh: RegistryListener = (payload) => {
      if (payload) {
        ++loadSequence.current;
        setDefinitions(payload.map(fromTitleTagDefinitionPayload));
        setError(false);
        setLoading(false);
      } else {
        void reload();
      }
    };
    listeners.add(refresh);
    return () => {
      listeners.delete(refresh);
    };
  }, [client, enabled, reload]);

  React.useEffect(() => {
    if (!enabled) {
      return;
    }
    void load("cache-first");
  }, [enabled, load]);

  return { definitions, loading, error, reload };
}
