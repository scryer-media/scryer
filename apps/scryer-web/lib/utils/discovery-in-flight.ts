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
