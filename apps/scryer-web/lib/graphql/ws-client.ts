import { createClient } from "graphql-ws";
import { getAuthlessWebClientProof } from "@/lib/authless-web-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeGraphqlUrl } from "@/lib/runtime-config";

function resolveWsUrl(): string {
  const graphqlUrl = getRuntimeGraphqlUrl();
  try {
    const url = new URL(graphqlUrl, window.location.origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = url.pathname.replace(/\/+$/, "") + "/ws";
    return url.toString();
  } catch {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}${graphqlUrl.replace(/\/+$/, "")}/ws`;
  }
}

export const wsClient = createClient({
  // Resolved on each connection attempt, not at import: modules that import
  // the client must still load where `window` does not exist.
  url: resolveWsUrl,
  connectionParams: async () => {
    const token = getAuthToken();
    if (token) {
      return { Authorization: `Bearer ${token}` };
    }
    const proof = await getAuthlessWebClientProof();
    return proof ? { authlessWebClientProof: proof } : {};
  },
  // Keep the connection alive for 3s after the last subscriber leaves.
  // Prevents React StrictMode's unmount/remount cycle from killing the
  // WS connection (lazyCloseTimeout defaults to 0 which closes immediately).
  lazyCloseTimeout: 3_000,
  retryAttempts: 5,
  retryWait: async (retries) => {
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(1000 * 2 ** retries, 30_000)),
    );
  },
});

export function disposeWsClient() {
  wsClient.dispose();
}
