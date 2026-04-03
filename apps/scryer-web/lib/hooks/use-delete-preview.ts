import * as React from "react";
import { useClient } from "urql";
import type { DeletePreview } from "@/lib/types/delete-preview";

type UseDeletePreviewResult = {
  preview: DeletePreview | null;
  loading: boolean;
  error: string | null;
};

export function useDeletePreview<TVariables extends Record<string, unknown>>(
  query: string,
  fieldName: string,
  variables: TVariables | null,
  enabled: boolean,
): UseDeletePreviewResult {
  const client = useClient();
  const [preview, setPreview] = React.useState<DeletePreview | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const variablesKey = React.useMemo(
    () => (variables ? JSON.stringify(variables) : ""),
    [variables],
  );
  const stableVariables = React.useMemo(
    () => (variablesKey ? (JSON.parse(variablesKey) as TVariables) : null),
    [variablesKey],
  );

  React.useEffect(() => {
    if (!enabled || !stableVariables) {
      setPreview(null);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void client
      .query(query, stableVariables, { requestPolicy: "network-only" })
      .toPromise()
      .then(({ data, error: queryError }) => {
        if (cancelled) {
          return;
        }
        if (queryError) {
          throw queryError;
        }

        const nextPreview = (data?.[fieldName] as DeletePreview | null | undefined) ?? null;
        if (!nextPreview) {
          throw new Error("delete preview payload missing");
        }

        setPreview(nextPreview);
      })
      .catch((nextError: unknown) => {
        if (cancelled) {
          return;
        }
        setPreview(null);
        setError(
          nextError instanceof Error ? nextError.message : String(nextError ?? "Unknown error"),
        );
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, enabled, fieldName, query, stableVariables]);

  return { preview, loading, error };
}
