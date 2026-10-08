import * as React from "react";
import { useClient, type Client } from "urql";

export type CanonicalVocabularyEntry = {
  key: string;
  name: string;
  category: string;
  aliases: string[];
};
type Vocabulary = { version: string; entries: CanonicalVocabularyEntry[] };
const query = `query CanonicalTagVocabulary($retry: Boolean!) {
  canonicalTagVocabulary(retry: $retry) { version entries { key name category aliases } }
}`;
const inFlight = new WeakMap<Client, Promise<Vocabulary>>();

function load(client: Client, retry: boolean): Promise<Vocabulary> {
  const current = inFlight.get(client);
  if (current) return current;
  const pending = client
    .query(query, { retry }, { requestPolicy: "network-only" })
    .toPromise()
    .then((result) => {
      if (result.error || !result.data?.canonicalTagVocabulary)
        throw result.error ?? new Error("Vocabulary unavailable");
      return result.data.canonicalTagVocabulary as Vocabulary;
    })
    .finally(() => inFlight.delete(client));
  inFlight.set(client, pending);
  return pending;
}

export function useCanonicalVocabulary() {
  const client = useClient();
  const [vocabulary, setVocabulary] = React.useState<Vocabulary | null>(null);
  const [error, setError] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const refresh = React.useCallback(
    async (retry: boolean) => {
      setLoading(true);
      try {
        setVocabulary(await load(client, retry));
        setError(false);
      } catch {
        setError(true);
      } finally {
        setLoading(false);
      }
    },
    [client],
  );
  React.useEffect(() => {
    void refresh(false);
  }, [refresh]);
  return { vocabulary, error, loading, retry: () => refresh(true) };
}
