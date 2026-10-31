import type { RegoValidationResult } from "./rego-diagnostics.ts";

export type ValidationSnapshot = {
  key: string;
  validating: boolean;
  result: RegoValidationResult | null;
};
export type ValidationClock = {
  set: (run: () => void, delay: number) => unknown;
  clear: (timer: unknown) => void;
};
const clock: ValidationClock = {
  set: (run, delay) => setTimeout(run, delay),
  clear: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
};

/** One editor session: one active request, one latest document, one cached result. */
export class RegoValidationSession {
  private generation = 0;
  private source = "";
  private key = "";
  private enabled = false;
  private timer: unknown;
  private active: Promise<RegoValidationResult> | null = null;
  private cached: { key: string; result: RegoValidationResult } | null = null;

  private validateSource: (source: string) => Promise<RegoValidationResult>;
  private publish: (snapshot: ValidationSnapshot) => void;
  private unavailableMessage: string;
  private timers: ValidationClock;

  constructor(
    validateSource: (source: string) => Promise<RegoValidationResult>,
    publish: (snapshot: ValidationSnapshot) => void,
    unavailableMessage: string,
    timers = clock,
  ) {
    this.validateSource = validateSource;
    this.publish = publish;
    this.unavailableMessage = unavailableMessage;
    this.timers = timers;
  }

  setDocument(source: string, key: string, enabled: boolean) {
    this.generation++;
    this.timers.clear(this.timer);
    this.source = source;
    this.key = key;
    this.enabled = enabled && Boolean(source.trim());
    this.publish({ key, validating: false, result: null });
    if (this.enabled)
      this.timer = this.timers.set(() => {
        void this.validate();
      }, 500);
  }

  async validate(): Promise<RegoValidationResult | null> {
    this.timers.clear(this.timer);
    if (!this.enabled) return null;
    const generation = this.generation;
    const key = this.key;
    // A new document waits for the bounded in-flight request; it does not
    // launch another request for each keystroke or keep a queue of old drafts.
    while (this.active) {
      await this.active;
      if (generation !== this.generation || !this.enabled) return null;
    }
    if (generation !== this.generation || !this.enabled) return null;
    if (this.cached?.key === key) {
      this.publish({ key, validating: false, result: this.cached.result });
      return this.cached.result;
    }
    this.publish({ key, validating: true, result: null });
    const source = this.source;
    const active = Promise.resolve()
      .then(() => this.validateSource(source))
      .catch(() => ({
        valid: false,
        errors: [this.unavailableMessage],
        unavailable: true,
      }));
    this.active = active;
    const result = await active;
    if (this.active === active) this.active = null;
    if (generation !== this.generation || !this.enabled) return null;
    if (!result.unavailable) this.cached = { key, result };
    this.publish({ key, validating: false, result });
    return result;
  }

  dispose() {
    this.generation++;
    this.enabled = false;
    this.cached = null;
    this.timers.clear(this.timer);
  }
}
