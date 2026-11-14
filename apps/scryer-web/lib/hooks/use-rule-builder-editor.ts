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
} from "@/lib/utils/rule-builder";
import {
  initialRuleEditorMode,
  withRuleAdvancedEditor,
  withRuleBuilderModel,
  withRuleSourceEdit,
  type RuleBuilderBinding,
  type RuleBuilderDraft,
  type RuleEditorMode,
} from "@/lib/utils/rule-builder-mode";

/// Catalogs a page can load for the builder's pickers, by option source.
export type RuleBuilderOptionLoaders = Readonly<
  Record<string, () => Promise<string[]>>
>;

/// The Simple/Advanced state one rule editor keeps, the same for every
/// family. Picker catalogs load the first time a chosen output needs them
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
  staticOptions?: Readonly<Record<string, readonly string[]>>;
  optionLoaders?: RuleBuilderOptionLoaders;
  onLoadError?: (error: unknown) => void;
}) {
  const [mode, setModeState] = useState<RuleEditorMode>("advanced");
  const [detached, setDetached] = useState(false);
  const [loaded, setLoaded] = useState<Record<string, string[]>>({});
  const loading = useRef(new Set<string>());
  const loaders = useRef(optionLoaders);
  const reportError = useRef(onLoadError);
  useEffect(() => {
    loaders.current = optionLoaders;
    reportError.current = onLoadError;
  });

  /// Every editor opening goes through here: a draft still made by the
  /// builder opens in it, anything else opens as Rego.
  const beginSession = useCallback((next: RuleBuilderDraft) => {
    setModeState(initialRuleEditorMode(next));
    setDetached(false);
  }, []);

  const editSource = useCallback(
    (source: string) => {
      setDraft((prev) => {
        const edit = withRuleSourceEdit(binding, prev, source);
        // Idempotent, so a replayed updater sets the same notice.
        if (edit.detached) setDetached(true);
        return edit.draft;
      });
    },
    [binding, setDraft],
  );

  const setMode = useCallback(
    (next: RuleEditorMode) => {
      if (next === "advanced")
        setDraft((prev) => withRuleAdvancedEditor(binding, prev));
      setModeState(next);
    },
    [binding, setDraft],
  );

  const setModel = useCallback(
    (model: RuleBuilderModel) =>
      setDraft((prev) => withRuleBuilderModel(binding, prev, model)),
    [binding, setDraft],
  );

  const builderShown =
    isEditorOpen && mode === "simple" && draft.builderModel !== null;
  const needed = builderShown
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
        .then((values) =>
          setLoaded((prev) => ({ ...prev, [source]: values })),
        )
        .catch((error: unknown) => {
          setLoaded((prev) => ({ ...prev, [source]: [] }));
          reportError.current?.(error);
        });
    }
  }, [neededKey, staticOptions]);

  const options = useMemo(() => {
    const all: Record<string, readonly string[] | null> = {};
    for (const source of neededKey ? neededKey.split("\n") : []) {
      all[source] = staticOptions?.[source] ?? loaded[source] ?? null;
    }
    return all;
  }, [loaded, neededKey, staticOptions]);

  return {
    mode,
    setMode,
    detached,
    beginSession,
    editSource,
    setModel,
    builderShown,
    options,
  };
}
