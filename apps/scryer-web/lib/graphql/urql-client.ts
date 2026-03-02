import {
  Client,
  fetchExchange,
  subscriptionExchange,
} from "@urql/core";
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
// Custom fetch — detects HTML responses (backend upgrade/restart splash) and
// re-throws with the body so the global-status-toast regex can catch it.
// ---------------------------------------------------------------------------

const scryerFetch: typeof fetch = async (input, init) => {
  const response = await fetch(input, init);
  const ct = response.headers.get("content-type") ?? "";
  if (ct.includes("text/html")) {
    const body = await response.text();
    throw new TypeError(body);
  }
  return response;
};

// ---------------------------------------------------------------------------
// Backend client — connects to the Rust GraphQL server at /graphql
// ---------------------------------------------------------------------------

export const backendClient = new Client({
  url: import.meta.env.SCRYER_GRAPHQL_URL ?? "/graphql",
  preferGetMethod: false,
  fetch: scryerFetch,
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
