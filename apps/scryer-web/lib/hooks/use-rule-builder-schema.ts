import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import { useClient, type Client } from "urql";
import { ruleBuilderSchemaQuery } from "@/lib/graphql/queries";
import type {
  RuleBuilderBaseSchema,
  RuleBuilderOutputDescriptor,
  RuleBuilderSchema,
  RuleBuilderSchemaPayload,
} from "@/lib/utils/rule-builder";
import {
  loadServedRuleBuilder,
  servedRuleBuilderSchema,
  servedRuleBuilderState,
  subscribeServedRuleBuilder,
} from "@/lib/utils/rule-builder-served";

export type RuleBuilderSchemaStatus = "loading" | "ready" | "failed";

function fetchServedSchema(client: Client) {
  return async (family: string): Promise<RuleBuilderSchemaPayload> => {
    const { data, error } = await client
      .query(ruleBuilderSchemaQuery, { family })
      .toPromise();
    if (error) throw error;
    const payload = data?.ruleBuilderSchema as RuleBuilderSchemaPayload | null;
    if (!payload) throw new Error("ruleBuilderSchema returned nothing");
    return payload;
  };
}

/// A family's builder schema with the outputs the server serves, read when the
/// page first mounts and kept for the page's lifetime. Null while loading and
/// after a failed read, which leaves only the Rego editor; `fallback` is used
/// instead when the read fails or serves no outputs.
export function useRuleBuilderSchema(
  base: RuleBuilderBaseSchema,
  fallback?: RuleBuilderSchema,
): {
  status: RuleBuilderSchemaStatus;
  schema: RuleBuilderSchema | null;
  /// The schema once the read settles, for an editor about to open.
  whenReady: () => Promise<RuleBuilderSchema | null>;
} {
  const client = useClient();
  const family = base.family;
  const snapshot = () => servedRuleBuilderState(family);
  // The store is module state, so a static render reads it as the page does.
  const state = useSyncExternalStore(subscribeServedRuleBuilder, snapshot, snapshot);
  const fetch = useMemo(() => fetchServedSchema(client), [client]);

  useEffect(() => {
    void loadServedRuleBuilder(family, fetch);
  }, [family, fetch]);

  const resolve = useCallback(
    (payload: RuleBuilderSchemaPayload | null) =>
      payload
        ? servedRuleBuilderSchema(base, payload, fallback)
        : (fallback ?? null),
    [base, fallback],
  );

  const schema = useMemo(
    () =>
      state?.status === "ready"
        ? resolve(state.payload)
        : state?.status === "failed"
          ? resolve(null)
          : null,
    [resolve, state],
  );

  const whenReady = useCallback(
    () => loadServedRuleBuilder(family, fetch).then(resolve),
    [family, fetch, resolve],
  );

  const status: RuleBuilderSchemaStatus =
    schema !== null ? "ready" : state?.status === "failed" ? "failed" : "loading";
  return { status, schema, whenReady };
}

const NO_DESCRIPTORS: readonly RuleBuilderOutputDescriptor[] = [];

/// The output descriptors the server serves for a family, for surfaces that
/// label heads (summaries, the Rego editor) rather than build rules. Empty
/// while the read is in flight and after it failed; heads then show by name.
/// Shares the one read per family the builder makes.
export function useRuleOutputDescriptors(
  family: string,
): readonly RuleBuilderOutputDescriptor[] {
  const client = useClient();
  const snapshot = () => servedRuleBuilderState(family);
  // The store is module state, so a static render reads it as the page does.
  const state = useSyncExternalStore(subscribeServedRuleBuilder, snapshot, snapshot);
  const fetch = useMemo(() => fetchServedSchema(client), [client]);
  useEffect(() => {
    void loadServedRuleBuilder(family, fetch);
  }, [family, fetch]);
  return state?.status === "ready" ? state.payload.outputs : NO_DESCRIPTORS;
}
