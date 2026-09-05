import * as React from "react";
import { useClient } from "urql";

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

  // The vocabulary is small, changes rarely, and is read by three surfaces at
  // once, so the first read goes through urql's document cache. An explicit
  // reload skips it: `deleteTitleTagDefinition` returns a payload type the
  // query never selects, so urql cannot invalidate the list on its own.
  const load = React.useCallback(
    async (requestPolicy: "cache-first" | "network-only") => {
      setLoading(true);
      try {
        const result = await client
          .query(titleTagDefinitionsQuery, {}, { requestPolicy })
          .toPromise();
        if (result.error) {
          throw result.error;
        }
        const payload = (result.data?.titleTagDefinitions ??
          []) as TitleTagDefinitionPayload[];
        setDefinitions(payload.map(fromTitleTagDefinitionPayload));
        setError(false);
      } catch {
        setDefinitions([]);
        setError(true);
      } finally {
        setLoading(false);
      }
    },
    [client],
  );

  const reload = React.useCallback(() => load("network-only"), [load]);

  React.useEffect(() => {
    if (!enabled) {
      return;
    }
    void load("cache-first");
  }, [enabled, load]);

  return { definitions, loading, error, reload };
}
