import * as React from "react";
import { useClient } from "urql";

import { titleTagDefinitionsQuery } from "@/lib/graphql/queries";
import type { TitleTagDefinition } from "@/lib/types/title-tags";

type TitleTagDefinitionPayload = {
  id: string;
  label: string;
  description?: string | null;
  titleCount: number;
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
 */
export function useTitleTagDefinitions() {
  const client = useClient();
  const [definitions, setDefinitions] = React.useState<TitleTagDefinition[]>([]);
  const [loading, setLoading] = React.useState(true);
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
    void load("cache-first");
  }, [load]);

  return { definitions, loading, error, reload };
}
