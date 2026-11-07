/** Shares pending work only; completed responses are never cached. */
export class DiscoveryInFlight<T> {
  private pending = new Map<string, Promise<T>>();

  run(key: string, load: () => Promise<T>, refresh = false): Promise<T> {
    const existing = this.pending.get(key);
    if (existing && !refresh) return existing;
    // A mutation needs a new read after the transport's pending query settles.
    const request = existing
      ? existing.then(load, load)
      : Promise.resolve().then(load);
    this.pending.set(key, request);
    const cleanup = () => {
      if (this.pending.get(key) === request) this.pending.delete(key);
    };
    void request.then(cleanup, cleanup);
    return request;
  }

  clear(): void {
    this.pending.clear();
  }
}

/**
 * Lets a response land only while it is the newest request of the current
 * scope. The scope is everything the response depends on (viewer,
 * permissions, facet, libraries), so a newer request or any scope change
 * leaves older responses stale, even ones sharing a coalesced request.
 */
export class DiscoveryResponseGate {
  private ticket = 0;
  private scope: string | null = null;

  begin(scope: string): number {
    this.ticket += 1;
    this.scope = scope;
    return this.ticket;
  }

  invalidate(): void {
    this.ticket += 1;
    this.scope = null;
  }

  accepts(ticket: number, scope: string): boolean {
    return ticket === this.ticket && scope === this.scope;
  }
}

/** The request scope a catalog Discovery response belongs to. */
export function catalogDiscoveryScope(
  authorizationSignature: string,
  input: unknown,
): string {
  return JSON.stringify([authorizationSignature, input]);
}
