import { useTranslate } from "../context/translate-context";
import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import * as React from "react";
import { useClient } from "urql";

import { seedingProfilesQuery } from "@/lib/graphql/queries";
import type { SeedingProfileOption } from "@/lib/types/seeding-profiles";

type SeedingProfileOptionsResult = {
  options: SeedingProfileOption[];
  loading: boolean;
  /** Verbatim server message when the list could not be loaded. */
  error: string | null;
  refresh: () => Promise<void>;
};

/**
 * Read-only seeding-profile list for the assignment dropdowns (indexer rows,
 * download-client routing entries). The editor surface owns writes; this hook
 * only supplies names for ids.
 */
export function useSeedingProfileOptions(): SeedingProfileOptionsResult {
  const t = useTranslate();
  const client = useClient();
  const [options, setOptions] = React.useState<SeedingProfileOption[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      // network-only: deleteSeedingProfile returns its own payload type, so the
      // document cache does not invalidate the list on delete.
      const result = await client
        .query(seedingProfilesQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error) throw result.error;
      const profiles: SeedingProfileOption[] = (
        result.data?.seedingProfiles ?? []
      ).map((profile: { id: string; name: string }) => ({
        id: profile.id,
        name: profile.name,
      }));
      setOptions(profiles);
      setError(null);
    } catch (err) {
      setError(userFacingGraphQlErrorMessage(err, t("status.failedToLoad"), t));
    } finally {
      setLoading(false);
    }
  }, [client, t]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  return { options, loading, error, refresh };
}
