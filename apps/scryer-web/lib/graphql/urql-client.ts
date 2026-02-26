import {
  Client,
  fetchExchange,
  subscriptionExchange,
} from "@urql/core";
import { persistedExchange } from "@urql/exchange-persisted";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { wsClient } from "@/lib/graphql/ws-client";

// ---------------------------------------------------------------------------
// Shared language ref — updated by the Provider when uiLanguage changes
// ---------------------------------------------------------------------------

let currentLanguage = "eng";

export function setGraphqlLanguage(lang: string) {
  currentLanguage = lang;
}

export function getGraphqlLanguage(): string {
  return currentLanguage;
}

// ---------------------------------------------------------------------------
// Backend client — connects to the Rust GraphQL server at /graphql
// ---------------------------------------------------------------------------

export const backendClient = new Client({
  url: import.meta.env.SCRYER_GRAPHQL_URL ?? "/graphql",
  preferGetMethod: false,
  exchanges: [
    subscriptionExchange({
      forwardSubscription(request) {
        const input = { ...request, query: request.query || "" };
        return {
          subscribe(sink) {
            const unsubscribe = wsClient.subscribe(input, sink);
            return { unsubscribe };
          },
        };
      },
    }),
    fetchExchange,
  ],
  fetchOptions: () => {
    const headers: Record<string, string> = {
      "x-scryer-language": currentLanguage,
    };
    const token = getAuthToken();
    if (token) {
      headers["authorization"] = `Bearer ${token}`;
    }
    return { headers };
  },
});

// ---------------------------------------------------------------------------
// SMG (Scryer Metadata Gateway) client — APQ + GET via persistedExchange
// ---------------------------------------------------------------------------

const smgUrl =
  import.meta.env.SCRYER_METADATA_GATEWAY_GRAPHQL_URL ??
  "http://127.0.0.1:8090/graphql";

export const smgClient = new Client({
  url: smgUrl,
  exchanges: [
    persistedExchange({
      preferGetForPersistedQueries: "within-url-limit",
    }),
    fetchExchange,
  ],
  fetchOptions: () => ({
    headers: {
      "x-scryer-language": currentLanguage,
    },
  }),
});
