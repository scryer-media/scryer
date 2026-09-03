import * as React from "react";
import { useClient } from "urql";
import type { DeletePreview } from "@/lib/types/delete-preview";

type UseDeletePreviewResult<TPayload = DeletePreview> = {
  preview: DeletePreview | null;
  /**
   * The raw root field payload. Queries whose root field is the preview itself
   * return the same object as `preview`; wrapper payloads (bulk previews) expose
   * their extra fields here.
   */
  payload: TPayload | null;
  loading: boolean;
  error: string | null;
};

export function useDeletePreview<
  TVariables extends Record<string, unknown>,
  TPayload = DeletePreview,
>(
  query: string,
  fieldName: string,
  variables: TVariables | null,
  enabled: boolean,
  /**
   * Pull the `DeletePreview` out of a wrapper payload. Defaults to treating the
   * root field as the preview itself.
   */
  selectPreview?: (payload: TPayload) => DeletePreview | null | undefined,
): UseDeletePreviewResult<TPayload> {
  const client = useClient();
  const [preview, setPreview] = React.useState<DeletePreview | null>(null);
  const [payload, setPayload] = React.useState<TPayload | null>(null);
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
  const selectPreviewRef = React.useRef(selectPreview);
  React.useEffect(() => {
    selectPreviewRef.current = selectPreview;
  }, [selectPreview]);

  React.useEffect(() => {
    if (!enabled || !stableVariables) {
      setPreview(null);
      setPayload(null);
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

        const nextPayload = (data?.[fieldName] as TPayload | null | undefined) ?? null;
        const select = selectPreviewRef.current;
        const nextPreview = nextPayload
          ? ((select
              ? select(nextPayload)
              : (nextPayload as unknown as DeletePreview)) ?? null)
          : null;
        if (!nextPreview) {
          throw new Error("delete preview payload missing");
        }

        setPayload(nextPayload);
        setPreview(nextPreview);
      })
      .catch((nextError: unknown) => {
        if (cancelled) {
          return;
        }
        setPreview(null);
        setPayload(null);
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

  return { preview, payload, loading, error };
}
