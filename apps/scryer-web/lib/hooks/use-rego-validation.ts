import { graphQlRuleValidationResult } from "../utils/rego-diagnostics";
import { userFacingGraphQlErrorMessage } from "../graphql/error-message";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
import { makeAbortableFetch } from "../graphql/urql-client";
import { useTranslate } from "../context/translate-context";
import {
  validateRuleSetMutation,
  validateRequestRuleMutation,
  validateMaintenanceRuleMutation,
} from "../graphql/mutations";
import type { RegoFamily } from "../utils/rego-assistance";
import type { RegoValidationResult } from "../utils/rego-diagnostics";
import {
  RegoValidationSession,
  type ValidationSnapshot,
} from "../utils/rego-validation";

const validators = {
  release: [validateRuleSetMutation, "validateRuleSet"],
  request: [validateRequestRuleMutation, "validateRequestRule"],
  maintenance: [validateMaintenanceRuleMutation, "validateMaintenanceRule"],
} as const;

export function useRegoValidation(
  family: RegoFamily,
  source: string,
  ruleId: string | null,
  open: boolean,
) {
  const client = useClient();
  const t = useTranslate();
  const unavailable = t("settings.regoValidationUnavailable");
  const key = JSON.stringify([family, ruleId, source, open]);
  const latestKey = useRef(key);
  const [snapshot, setSnapshot] = useState<ValidationSnapshot>({
    key: "",
    validating: false,
    result: null,
  });
  const session = useMemo(
    () =>
      new RegoValidationSession(
        async (regoSource) => {
          const [mutation, field] = validators[family];
          const abort = new AbortController();
          const timeout = setTimeout(() => abort.abort(), 15_000);
          try {
            const { data, error } = await client
              .mutation(
                mutation,
                {
                  input: {
                    regoSource,
                    ...(family === "release" && ruleId
                      ? { ruleSetId: ruleId }
                      : {}),
                  },
                },
                { fetch: makeAbortableFetch(abort.signal) },
              )
              .toPromise();
            if (error?.networkError) throw error;
            if (error?.graphQLErrors.length) {
              return graphQlRuleValidationResult(
                error,
                userFacingGraphQlErrorMessage(
                  error,
                  t("status.failedToUpdate"),
                  t,
                ),
              );
            }
            if (!data?.[field]) throw new Error("Missing validation response");
            return data[field] as RegoValidationResult;
          } finally {
            clearTimeout(timeout);
          }
        },
        setSnapshot,
        unavailable,
      ),
    [client, family, ruleId, unavailable, t],
  );

  useLayoutEffect(() => {
    latestKey.current = key;
    session.setDocument(source, key, open);
    return () => session.dispose();
  }, [session, source, key, open]);

  const validateDraft = useCallback(() => session.validate(), [session]);
  // Existing form-level checks (name, scope, save failures) share the summary.
  const setValidationResult = useCallback(
    (result: RegoValidationResult | null) => {
      setSnapshot({ key: latestKey.current, validating: false, result });
    },
    [],
  );
  return {
    validateDraft,
    setValidationResult,
    validationResult: snapshot.key === key ? snapshot.result : null,
    validating: snapshot.key === key && snapshot.validating,
  };
}
