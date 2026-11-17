import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ruleBuilderOptionSources,
  type RuleBuilderModel,
  type RuleBuilderOptionValue,
} from "@/lib/utils/rule-builder";
import {
  initialRuleEditorMode,
  nextRuleEditorMode,
  withRuleBuilderModel,
  withRuleRegoEditor,
  withRuleSourceEdit,
  type RuleBuilderBinding,
  type RuleBuilderDraft,
  type RuleEditorMode,
} from "@/lib/utils/rule-builder-mode";

/// Catalogs a page can load for the builder's pickers, by option source.
export type RuleBuilderOptionLoaders = Readonly<
  Record<string, () => Promise<readonly RuleBuilderOptionValue[]>>
>;

/// The Simple, Advanced view and Advanced edit state one rule editor keeps,
/// the same for every family. Picker catalogs load the first time a chosen output needs them
/// while the builder is shown, and are kept for the page's lifetime.
export function useRuleBuilderEditor<D extends RuleBuilderDraft>({
  binding,
  draft,
  setDraft,
  isEditorOpen,
  staticOptions,
  optionLoaders,
  onLoadError,
}: {
  binding: RuleBuilderBinding<D>;
  draft: D;
  setDraft: Dispatch<SetStateAction<D>>;
  isEditorOpen: boolean;
  /// Catalogs the page already holds.
  staticOptions?: Readonly<Record<string, readonly RuleBuilderOptionValue[]>>;
  optionLoaders?: RuleBuilderOptionLoaders;
  onLoadError?: (error: unknown) => void;
}) {
  const [mode, setModeState] = useState<RuleEditorMode>("edit");
  const [loaded, setLoaded] = useState<
    Record<string, readonly RuleBuilderOptionValue[]>
  >({});
  const loading = useRef(new Set<string>());
  const loaders = useRef(optionLoaders);
  const reportError = useRef(onLoadError);
  useEffect(() => {
    loaders.current = optionLoaders;
    reportError.current = onLoadError;
  });

  /// Every editor opening goes through here: a draft still made by the
  /// builder opens in it, anything else opens in the Rego editor.
  const beginSession = useCallback((next: RuleBuilderDraft) => {
    setModeState(initialRuleEditorMode(next));
  }, []);

  const editSource = useCallback(
    (source: string) => setDraft((prev) => withRuleSourceEdit(prev, source)),
    [setDraft],
  );

  /// Move between the builder and its read-only Rego. Never leaves `edit`.
  const setMode = useCallback(
    (next: "simple" | "view") =>
      setModeState((current) => nextRuleEditorMode(draft, current, next)),
    [draft],
  );

  /// Detach the draft and open the Rego editor. Callers confirm first when
  /// `ruleRegoEditorNeedsConfirmation` says so.
  const editAsRego = useCallback(() => {
    setDraft((prev) => withRuleRegoEditor(binding, prev));
    setModeState("edit");
  }, [binding, setDraft]);

  const setModel = useCallback(
    (model: RuleBuilderModel) =>
      setDraft((prev) => withRuleBuilderModel(binding, prev, model)),
    [binding, setDraft],
  );

  const builderShown =
    isEditorOpen && mode === "simple" && draft.builderModel !== null;
  /// The builder still owns the source, whether shown as the builder or as
  /// read-only Rego, so a save is held to what the builder can write.
  const attached =
    isEditorOpen && mode !== "edit" && draft.builderModel !== null;
  const needed =
    builderShown && binding.schema
      ? ruleBuilderOptionSources(binding.schema, draft.builderModel)
      : [];
  const neededKey = needed.join("\n");

  useEffect(() => {
    if (!neededKey) return;
    for (const source of neededKey.split("\n")) {
      const load = loaders.current?.[source];
      if (!load || staticOptions?.[source] || loading.current.has(source))
        continue;
      loading.current.add(source);
      load()
        .then((values) => setLoaded((prev) => ({ ...prev, [source]: values })))
        .catch((error: unknown) => {
          setLoaded((prev) => ({ ...prev, [source]: [] }));
          reportError.current?.(error);
        });
    }
  }, [neededKey, staticOptions]);

  /// Only catalogs the page can supply are listed; the builder types a value
  /// for any other catalog in as free text.
  const options = useMemo(() => {
    const all: Record<string, readonly RuleBuilderOptionValue[] | null> = {};
    for (const source of neededKey ? neededKey.split("\n") : []) {
      const known = staticOptions?.[source];
      if (known) all[source] = known;
      else if (optionLoaders?.[source]) all[source] = loaded[source] ?? null;
    }
    return all;
  }, [loaded, neededKey, optionLoaders, staticOptions]);

  return {
    mode,
    setMode,
    editAsRego,
    beginSession,
    editSource,
    setModel,
    builderShown,
    attached,
    options,
  };
}
