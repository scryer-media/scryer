/** Shares pending work only; completed responses are never cached. */
export class DiscoveryInFlight<T> {
  private pending = new Map<string, Promise<T>>();

  run(key: string, load: () => Promise<T>): Promise<T> {
    const existing = this.pending.get(key);
    if (existing) return existing;
    const request = Promise.resolve().then(load);
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
