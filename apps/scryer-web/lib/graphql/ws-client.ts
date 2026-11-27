import { createClient } from "graphql-ws";
import { getAuthlessWebClientProof } from "@/lib/authless-web-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { createWsLiveness } from "@/lib/graphql/ws-liveness";

const WS_KEEP_ALIVE_MS = 20_000;
const WS_PONG_TIMEOUT_MS = 5_000;

const liveness = createWsLiveness({ pongTimeoutMs: WS_PONG_TIMEOUT_MS });

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
  // A ping that goes unanswered closes the socket so it reconnects; without
  // one, a connection a sleeping phone dropped looks open forever.
  keepAlive: WS_KEEP_ALIVE_MS,
  // An installed app cannot be reloaded, so the socket never gives up: it
  // keeps retrying with the capped backoff below, network errors included.
  retryAttempts: Infinity,
  shouldRetry: () => true,
  retryWait: async (retries) => {
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(1000 * 2 ** retries, 30_000)),
    );
  },
  on: {
    connected: (socket, _payload, wasRetry) =>
      liveness.connected(socket, wasRetry),
    closed: () => liveness.closed(),
    ping: (received) => liveness.ping(received),
    pong: (received) => liveness.pong(received),
  },
});

if (typeof window !== "undefined") {
  // Coming back to the foreground or back online is when a dead socket shows:
  // check it now rather than at the next keep-alive.
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") liveness.probe();
  });
  window.addEventListener("online", () => liveness.probe());
}

/**
 * Runs `listener` each time the socket reconnects after a drop. Events sent
 * while it was down were lost, so a subscriber reads its state again.
 */
export function onWsReconnected(listener: () => void): () => void {
  return liveness.onReconnected(listener);
}

export function disposeWsClient() {
  wsClient.dispose();
}
