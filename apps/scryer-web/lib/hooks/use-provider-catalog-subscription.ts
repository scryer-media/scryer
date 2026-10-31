import { useEffect, useRef } from "react";

import { providerCatalogChangedSubscription } from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

const PROVIDER_CATALOG_FAMILIES = new Set([
  "SUBTITLE",
  "NOTIFICATION",
  "INDEXER",
  "DOWNLOAD_CLIENT",
  "ARCHIVE_EXTRACTOR",
] as const);

export type ProviderCatalogFamily =
  | "SUBTITLE"
  | "NOTIFICATION"
  | "INDEXER"
  | "DOWNLOAD_CLIENT"
  | "ARCHIVE_EXTRACTOR";

function isProviderCatalogFamily(
  value: string,
): value is ProviderCatalogFamily {
  return PROVIDER_CATALOG_FAMILIES.has(value as ProviderCatalogFamily);
}

export function useProviderCatalogSubscription(
  onChanged: (families: ProviderCatalogFamily[]) => void,
  enabled = true,
) {
  const onChangedRef = useRef(onChanged);

  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription<{
    data?: { providerCatalogChanged?: ProviderCatalogFamily[] };
  }>({
    enabled,
    requestKey: "providerCatalogChanged",
    request: { query: providerCatalogChangedSubscription },
    onNext(result) {
      const families =
        result.data?.providerCatalogChanged?.filter(isProviderCatalogFamily) ??
        [];
      if (families.length > 0) {
        onChangedRef.current(families);
      }
    },
    onError(err) {
      console.error("[provider-catalog-changed] subscription error:", err);
    },
  });
}
