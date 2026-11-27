/**
 * Keeps the event socket honest on devices that suspend the page. A phone that
 * sleeps can drop the TCP connection without the socket ever seeing a close,
 * so the page keeps waiting on a connection that delivers nothing. Each ping
 * must be answered within `pongTimeoutMs`, or the socket is closed so the
 * client reconnects. Listeners hear about every reconnect, because whatever
 * happened while the socket was down was never delivered and has to be read.
 */

type SocketLike = {
  readyState: number;
  send: (data: string) => void;
  close: (code?: number, reason?: string) => void;
};

type Timers = {
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
};

const OPEN = 1;
export const WS_PONG_TIMEOUT_CLOSE_CODE = 4408;

export function createWsLiveness({
  pongTimeoutMs,
  timers = {
    setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
    clearTimeout: (handle) =>
      globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
  },
}: {
  pongTimeoutMs: number;
  timers?: Timers;
}) {
  let socket: SocketLike | null = null;
  let pongTimer: unknown = null;
  const reconnectListeners = new Set<() => void>();

  const clearPongTimer = () => {
    if (pongTimer !== null) {
      timers.clearTimeout(pongTimer);
      pongTimer = null;
    }
  };

  const awaitPong = () => {
    clearPongTimer();
    const watched = socket;
    pongTimer = timers.setTimeout(() => {
      pongTimer = null;
      if (watched && watched === socket && watched.readyState === OPEN) {
        watched.close(WS_PONG_TIMEOUT_CLOSE_CODE, "Pong timeout");
      }
    }, pongTimeoutMs);
  };

  return {
    connected(next: unknown, wasRetry: boolean) {
      socket = next as SocketLike;
      clearPongTimer();
      if (wasRetry) {
        for (const listener of [...reconnectListeners]) listener();
      }
    },
    closed() {
      socket = null;
      clearPongTimer();
    },
    /** The client sent (`received` false) or got a ping. */
    ping(received: boolean) {
      if (!received) awaitPong();
    },
    pong(received: boolean) {
      if (received) clearPongTimer();
    },
    /**
     * Ping now instead of waiting for the next keep-alive, for when the page
     * comes back to the foreground and the socket may have died meanwhile.
     */
    probe() {
      if (!socket || socket.readyState !== OPEN || pongTimer !== null) return;
      try {
        socket.send(JSON.stringify({ type: "ping" }));
      } catch {
        socket.close(WS_PONG_TIMEOUT_CLOSE_CODE, "Ping failed");
        return;
      }
      awaitPong();
    },
    onReconnected(listener: () => void) {
      reconnectListeners.add(listener);
      return () => {
        reconnectListeners.delete(listener);
      };
    },
  };
}
