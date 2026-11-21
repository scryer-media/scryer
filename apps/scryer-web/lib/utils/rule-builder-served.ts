import {
  ruleBuilderOutputsFromPayload,
  ruleBuilderSchemaFromPayload,
  type RuleBuilderBaseSchema,
  type RuleBuilderOutput,
  type RuleBuilderSchema,
  type RuleBuilderSchemaPayload,
} from "./rule-builder.ts";

/// The outputs each rule family's builder offers, as the server last served
/// them. Read once per family and kept for the page's lifetime; a failed read
/// is not kept, so the next page that needs it asks again.

export type ServedRuleBuilderState =
  | { status: "loading" }
  | {
      status: "ready";
      payload: RuleBuilderSchemaPayload;
      outputs: RuleBuilderOutput[];
    }
  | { status: "failed" };

const states = new Map<string, ServedRuleBuilderState>();
const pending = new Map<string, Promise<RuleBuilderSchemaPayload | null>>();
const listeners = new Set<() => void>();

function publish(family: string, state: ServedRuleBuilderState) {
  states.set(family, state);
  for (const listener of listeners) listener();
}

export function subscribeServedRuleBuilder(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/// What is known of a family's outputs, without asking the server.
export function servedRuleBuilderState(
  family: string,
): ServedRuleBuilderState | null {
  return states.get(family) ?? null;
}

/// The served outputs of a family, or null before they are read.
export function servedRuleBuilderOutputs(
  family: string,
): RuleBuilderOutput[] | null {
  const state = states.get(family);
  return state?.status === "ready" ? state.outputs : null;
}

/// Read a family's outputs once. Callers share one read; a family already
/// read resolves at once, and one whose read failed is asked again.
export function loadServedRuleBuilder(
  family: string,
  fetch: (family: string) => Promise<RuleBuilderSchemaPayload>,
): Promise<RuleBuilderSchemaPayload | null> {
  const state = states.get(family);
  if (state?.status === "ready") return Promise.resolve(state.payload);
  const inFlight = pending.get(family);
  if (inFlight) return inFlight;
  publish(family, { status: "loading" });
  const load = fetch(family)
    .then((payload) => {
      publish(family, {
        status: "ready",
        payload,
        outputs: ruleBuilderOutputsFromPayload(payload).outputs,
      });
      return payload;
    })
    .catch(() => {
      publish(family, { status: "failed" });
      return null;
    })
    .finally(() => pending.delete(family));
  pending.set(family, load);
  return load;
}

const resolved = new WeakMap<
  RuleBuilderSchemaPayload,
  WeakMap<RuleBuilderBaseSchema, RuleBuilderSchema>
>();

/// A family's whole schema from what the server served, the same object for
/// the same payload so everything keyed on a schema stays cached. `fallback`
/// is used when the server serves no outputs at all, as scoring may.
export function servedRuleBuilderSchema(
  base: RuleBuilderBaseSchema,
  payload: RuleBuilderSchemaPayload,
  fallback?: RuleBuilderSchema,
): RuleBuilderSchema {
  if (fallback && payload.outputs.length === 0) return fallback;
  let byBase = resolved.get(payload);
  if (!byBase) {
    byBase = new WeakMap();
    resolved.set(payload, byBase);
  }
  let schema = byBase.get(base);
  if (!schema) {
    schema = ruleBuilderSchemaFromPayload(base, payload);
    byBase.set(base, schema);
  }
  return schema;
}

/// Forget everything read. Tests only.
export function resetServedRuleBuilder(): void {
  states.clear();
  pending.clear();
  for (const listener of listeners) listener();
}
