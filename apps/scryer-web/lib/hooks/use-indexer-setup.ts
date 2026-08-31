import { useCallback, useEffect, useMemo, useState } from "react";
import type { Client } from "urql";

import {
  createIndexerMutation,
  testIndexerConnectionMutation,
} from "@/lib/graphql/mutations";
import {
  applyIndexerConfigOption,
  buildSetupIndexerConfigValues,
  findMissingSetupIndexerField,
  serializeSetupIndexerConfigValues,
} from "@/lib/utils/indexer-setup";
import type { ConfigFieldDef } from "@/lib/types";

type TranslateFn = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

export type SetupIndexerProviderOption = {
  value: string;
  label: string;
  defaultBaseUrl?: string;
  configFields: ConfigFieldDef[];
};

interface UseIndexerSetupArgs {
  client: Client;
  t: TranslateFn;
}

export function useIndexerSetup({ client, t }: UseIndexerSetupArgs) {
  // ── Step 5 (fresh): Indexer ─────────────────────────────────────────
  const [idxName, setIdxName] = useState("");
  const [idxProviderType, setIdxProviderType] = useState("");
  const [idxConfigValues, setIdxConfigValues] = useState<
    Record<string, string>
  >({});
  const [idxProviderOptions, setIdxProviderOptions] = useState<
    SetupIndexerProviderOption[]
  >([]);
  const [idxTesting, setIdxTesting] = useState(false);
  const [idxTestResult, setIdxTestResult] = useState<
    "success" | "failed" | null
  >(null);
  const [idxSaving, setIdxSaving] = useState(false);
  const [idxSaved, setIdxSaved] = useState(false);
  const [idxError, setIdxError] = useState<string | null>(null);

  useEffect(() => {
    if (idxProviderOptions.some((option) => option.value === idxProviderType)) {
      return;
    }
    const firstProvider = idxProviderOptions[0];
    if (firstProvider?.value) {
      setIdxProviderType(firstProvider.value);
      setIdxConfigValues(
        buildSetupIndexerConfigValues(firstProvider.configFields),
      );
      setIdxName((current) => current || firstProvider.label);
    }
  }, [idxProviderOptions, idxProviderType]);

  const selectedIdxProvider = useMemo(
    () =>
      idxProviderOptions.find((option) => option.value === idxProviderType) ??
      null,
    [idxProviderOptions, idxProviderType],
  );
  const selectedIdxProviderFields = useMemo(
    () => selectedIdxProvider?.configFields ?? [],
    [selectedIdxProvider],
  );
  const indexerProviderConfigFieldsByType = useMemo(
    () =>
      new Map(
        idxProviderOptions.map(
          (option) => [option.value, option.configFields] as const,
        ),
      ),
    [idxProviderOptions],
  );

  const resetIndexerSavedState = useCallback(() => {
    setIdxSaved(false);
    setIdxTestResult(null);
    setIdxError(null);
  }, []);

  const handleIdxNameChange = useCallback(
    (value: string) => {
      setIdxName(value);
      resetIndexerSavedState();
    },
    [resetIndexerSavedState],
  );

  const handleIdxProviderTypeChange = useCallback(
    (nextProviderType: string) => {
      const nextProvider =
        idxProviderOptions.find(
          (option) => option.value === nextProviderType,
        ) ?? null;
      setIdxProviderType(nextProviderType);
      setIdxConfigValues(
        buildSetupIndexerConfigValues(nextProvider?.configFields ?? []),
      );
      setIdxName((current) => current || nextProvider?.label || "");
      resetIndexerSavedState();
    },
    [idxProviderOptions, resetIndexerSavedState],
  );

  const handleIdxConfigValueChange = useCallback(
    (key: string, value: string) => {
      setIdxConfigValues((current) =>
        applyIndexerConfigOption(
          selectedIdxProviderFields,
          current,
          key,
          value,
        ),
      );
      resetIndexerSavedState();
    },
    [resetIndexerSavedState, selectedIdxProviderFields],
  );

  const buildIndexerConfigValues = useCallback(() => {
    if (!idxProviderType) {
      setIdxError(t("form.providerTypePlaceholder"));
      return null;
    }

    const missingField = findMissingSetupIndexerField(
      selectedIdxProviderFields,
      idxConfigValues,
    );
    if (missingField) {
      setIdxError(`${missingField.label}: ${t("setup.required")}`);
      return null;
    }

    return serializeSetupIndexerConfigValues(
      selectedIdxProviderFields,
      idxConfigValues,
    );
  }, [idxConfigValues, idxProviderType, selectedIdxProviderFields, t]);

  // ── Indexer test ────────────────────────────────────────────────────
  const testIndexer = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const config = buildIndexerConfigValues();
    if (config === null) {
      setIdxTesting(false);
      return;
    }
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            providerType: idxProviderType,
            config,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection?.status === "ok") {
        setIdxTestResult("success");
      } else {
        setIdxTestResult("failed");
      }
    } catch {
      setIdxTestResult("failed");
    } finally {
      setIdxTesting(false);
    }
  }, [buildIndexerConfigValues, client, idxProviderType]);

  // ── Indexer save ────────────────────────────────────────────────────
  const saveIndexer = useCallback(async () => {
    setIdxSaving(true);
    setIdxError(null);
    const config = buildIndexerConfigValues();
    if (config === null) {
      setIdxSaving(false);
      return;
    }
    try {
      const { error } = await client
        .mutation(createIndexerMutation, {
          input: {
            name: idxName.trim(),
            providerType: idxProviderType,
            config,
            isEnabled: true,
            enableInteractiveSearch: true,
            enableAutoSearch: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setIdxSaved(true);
    } catch (err) {
      setIdxError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setIdxSaving(false);
    }
  }, [buildIndexerConfigValues, client, idxName, idxProviderType]);

  const handleIdxTestAndSave = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const config = buildIndexerConfigValues();
    if (config === null) {
      setIdxTesting(false);
      return;
    }
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            providerType: idxProviderType,
            config,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection?.status === "ok") {
        setIdxTestResult("success");
        setIdxTesting(false);
        await saveIndexer();
      } else {
        setIdxTestResult("failed");
        setIdxTesting(false);
      }
    } catch {
      setIdxTestResult("failed");
      setIdxTesting(false);
    }
  }, [buildIndexerConfigValues, client, idxProviderType, saveIndexer]);

  return {
    idxName,
    idxProviderType,
    idxConfigValues,
    idxProviderOptions,
    setIdxProviderOptions,
    idxTesting,
    idxTestResult,
    idxSaving,
    idxSaved,
    idxError,
    indexerProviderConfigFieldsByType,
    handleIdxNameChange,
    handleIdxProviderTypeChange,
    handleIdxConfigValueChange,
    testIndexer,
    handleIdxTestAndSave,
  };
}
