import { useEffect, useRef, useState } from "react";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  lineNumbers,
  keymap,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import {
  Compartment,
  EditorState,
  StateEffect,
  StateField,
  type Extension,
  type Range,
} from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/legacy-modes/mode/javascript";
import { xml } from "@codemirror/legacy-modes/mode/xml";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { defaultHighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { setDiagnostics as setLintDiagnostics } from "@codemirror/lint";
import { regoAssistance } from "./rego-editor-assistance";
import { formatScriptIndentation, scriptAssistance, scriptLanguageExtensions, type ScriptEditorLanguage } from "./script-editor-assistance";
import type { ScriptTriggerValue } from "@/lib/types/scripts";
import type { RegoFamily } from "@/lib/utils/rego-assistance";
import { useTheme } from "next-themes";
import { Check, Copy } from "lucide-react";
import "@fontsource-variable/jetbrains-mono";
import { IconButton } from "@/components/ui/icon-button";
import { Button } from "@/components/ui/button";
import { CODE_FONT } from "@/lib/fonts";
import { isDarkTheme } from "@/lib/theme";

export type CodeEditorLanguage = "plain" | "javascript" | "json" | "rego" | "xml" | ScriptEditorLanguage;

export type CodeEditorDiagnostic = {
  line: number;
  column?: number | null;
  message?: string;
};

export type CodeEditorProps = {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
  minLines?: number;
  maxLines?: number;
  diagnostics?: CodeEditorDiagnostic[];
  language?: CodeEditorLanguage;
  regoFamily?: RegoFamily;
  regoTranslate?: (key: string) => string;
  scriptTrigger?: ScriptTriggerValue;
  scriptTranslate?: (key: string) => string;
  copyable?: boolean;
  copyLabel?: string;
  copiedLabel?: string;
  autoFocus?: boolean;
  onAutoFocus?: () => void;
};

const regoKeywords = new Set([
  "as",
  "contains",
  "default",
  "else",
  "every",
  "false",
  "if",
  "import",
  "in",
  "not",
  "null",
  "package",
  "some",
  "true",
  "with",
]);

const regoBuiltins = new Set([
  "data",
  "input",
  "is_array",
  "is_boolean",
  "is_null",
  "is_number",
  "is_object",
  "is_set",
  "is_string",
  "lower",
  "replace",
  "round",
  "split",
  "sprintf",
  "startswith",
  "sum",
  "time",
  "to_number",
  "trim",
  "type_name",
  "upper",
]);

const regoHighlightTheme = EditorView.theme({
  ".cm-scryer-rego-keyword": {
    color: "#c4b5fd",
    fontWeight: "700",
  },
  ".cm-scryer-rego-builtin": {
    color: "#7dd3fc",
    fontWeight: "600",
  },
  ".cm-scryer-rego-variable": {
    color: "#93c5fd",
  },
  ".cm-scryer-rego-property": {
    color: "#d8b4fe",
  },
  ".cm-scryer-rego-string": {
    color: "#86efac",
  },
  ".cm-scryer-rego-number": {
    color: "#fde68a",
  },
  ".cm-scryer-rego-comment": {
    color: "#94a3b8",
    fontStyle: "italic",
  },
  ".cm-scryer-rego-operator": {
    color: "#f0abfc",
  },
  ".light & .cm-scryer-rego-keyword": {
    color: "#6d28d9",
  },
  ".light & .cm-scryer-rego-builtin": {
    color: "#0369a1",
  },
  ".light & .cm-scryer-rego-variable": {
    color: "#1d4ed8",
  },
  ".light & .cm-scryer-rego-property": {
    color: "#7e22ce",
  },
  ".light & .cm-scryer-rego-string": {
    color: "#047857",
  },
  ".light & .cm-scryer-rego-number": {
    color: "#b45309",
  },
  ".light & .cm-scryer-rego-comment": {
    color: "#64748b",
  },
  ".light & .cm-scryer-rego-operator": {
    color: "#a21caf",
  },
});

function regoDecoration(className: string): Decoration {
  return Decoration.mark({ class: className });
}

function buildRegoHighlightDecorations(state: EditorState): DecorationSet {
  const ranges: Range<Decoration>[] = [];

  for (let lineNumber = 1; lineNumber <= state.doc.lines; lineNumber += 1) {
    const line = state.doc.line(lineNumber);
    const text = line.text;
    let index = 0;

    while (index < text.length) {
      const char = text[index];
      if (!char || /\s/.test(char)) {
        index += 1;
        continue;
      }

      const from = line.from + index;
      if (char === "#") {
        ranges.push(regoDecoration("cm-scryer-rego-comment").range(from, line.to));
        break;
      }

      if (char === "\"" || char === "`") {
        const quote = char;
        let end = index + 1;
        while (end < text.length) {
          const current = text[end];
          if (current === "\\" && quote === "\"") {
            end += 2;
            continue;
          }
          end += 1;
          if (current === quote) {
            break;
          }
        }
        ranges.push(
          regoDecoration("cm-scryer-rego-string").range(from, line.from + end),
        );
        index = end;
        continue;
      }

      const number = text.slice(index).match(/^\d+(?:\.\d+)?/);
      if (number) {
        const end = index + number[0].length;
        ranges.push(
          regoDecoration("cm-scryer-rego-number").range(from, line.from + end),
        );
        index = end;
        continue;
      }

      const identifier = text.slice(index).match(/^[A-Za-z_][A-Za-z0-9_]*/);
      if (identifier) {
        const value = identifier[0];
        const end = index + value.length;
        const previousChar = index > 0 ? text[index - 1] : "";
        const className = regoKeywords.has(value)
          ? "cm-scryer-rego-keyword"
          : regoBuiltins.has(value)
            ? "cm-scryer-rego-builtin"
            : previousChar === "."
              ? "cm-scryer-rego-property"
              : "cm-scryer-rego-variable";
        ranges.push(regoDecoration(className).range(from, line.from + end));
        index = end;
        continue;
      }

      const operator = text.slice(index).match(/^(?:==|!=|<=|>=|:=|[_+\-*/%<>=|&!]+|[{}()[\],.;:])/);
      if (operator) {
        const end = index + operator[0].length;
        ranges.push(
          regoDecoration("cm-scryer-rego-operator").range(from, line.from + end),
        );
        index = end;
        continue;
      }

      index += 1;
    }
  }

  return Decoration.set(ranges, true);
}

const regoHighlightPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildRegoHighlightDecorations(view.state);
    }

    update(update: ViewUpdate) {
      if (update.docChanged) {
        this.decorations = buildRegoHighlightDecorations(update.state);
      }
    }
  },
  {
    decorations: (plugin) => plugin.decorations,
  },
);

const setDiagnosticsEffect = StateEffect.define<CodeEditorDiagnostic[]>();

function diagnosticDecorations(
  state: EditorState,
  diagnostics: CodeEditorDiagnostic[],
): DecorationSet {
  const decorations = diagnostics
    .filter((diagnostic) => Number.isFinite(diagnostic.line))
    .sort((a, b) => a.line - b.line)
    .flatMap((diagnostic) => {
      if (diagnostic.line < 1 || diagnostic.line > state.doc.lines) {
        return [];
      }

      const line = state.doc.line(diagnostic.line);
      const column = diagnostic.column && Number.isFinite(diagnostic.column)
        ? Math.max(1, diagnostic.column)
        : 1;
      const from = Math.min(line.to, line.from + column - 1);
      const to = Math.max(from + 1, Math.min(line.to, line.to));
      const attributes = diagnostic.message ? { title: diagnostic.message } : undefined;

      return [
        Decoration.line({ class: "cm-code-diagnostic-line" }).range(line.from),
        Decoration.mark({ attributes, class: "cm-code-diagnostic" }).range(from, to),
      ];
    });

  return Decoration.set(decorations, true);
}

const diagnosticField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(decorations, transaction) {
    for (const effect of transaction.effects) {
      if (effect.is(setDiagnosticsEffect)) {
        return diagnosticDecorations(transaction.state, effect.value);
      }
    }

    if (transaction.docChanged) {
      return decorations.map(transaction.changes);
    }

    return decorations;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const diagnosticTheme = EditorView.baseTheme({
  ".cm-code-diagnostic-line": {
    backgroundColor: "rgba(255, 88, 112, 0.08)",
  },
  ".cm-code-diagnostic": {
    textDecorationColor: "var(--scry-danger-text)",
    textDecorationLine: "underline",
    textDecorationSkipInk: "none",
    textDecorationStyle: "wavy",
    textDecorationThickness: "1.5px",
    textUnderlineOffset: "3px",
  },
});

const lightTheme = EditorView.theme({
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)" },
  ".cm-gutters": { backgroundColor: "var(--muted)", borderRight: "1px solid var(--border-color)", paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "var(--ring)", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--ring)" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(var(--scry-accent-rgb), 0.2)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(var(--scry-accent-rgb), 0.3)" },
  ".cm-gutters .cm-gutter": { fontFamily: CODE_FONT },
});

const scryerDark = EditorView.theme({
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "var(--ring)", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--ring)" },
  ".cm-gutters": { backgroundColor: "var(--background)", color: "var(--muted-foreground)", borderRight: "1px solid var(--border-color)", fontFamily: CODE_FONT, paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)", color: "var(--foreground)" },
  ".cm-activeLine": { backgroundColor: "rgba(var(--scry-accent-rgb), 0.06)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(var(--scry-accent-rgb), 0.2)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(var(--scry-accent-rgb), 0.3)" },
  ".cm-line": { padding: "0 4px" },
}, { dark: true });

function languageExtensions(language: CodeEditorLanguage): Extension[] {
  if (language === "javascript") {
    return [javascript()];
  }

  if (language === "json") {
    return [StreamLanguage.define(json)];
  }

  if (["shell", "python", "powershell", "batch", "go"].includes(language)) {
    return [scriptLanguageExtensions(language as ScriptEditorLanguage)];
  }

  if (language === "xml") {
    return [StreamLanguage.define(xml)];
  }

  return [];
}

export default function CodeEditor({
  id,
  value,
  onChange,
  readOnly = false,
  height = "320px",
  minLines,
  maxLines,
  diagnostics = [],
  language = "plain",
  regoFamily,
  regoTranslate,
  scriptTrigger,
  scriptTranslate,
  copyable = false,
  copyLabel = "Copy code",
  copiedLabel = "Copied",
  autoFocus = false,
  onAutoFocus,
}: CodeEditorProps) {
  const [assistance] = useState(() => new Compartment());
  const [appearance] = useState(() => new Compartment());
  const [editable] = useState(() => new Compartment());
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const copyResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [copied, setCopied] = useState(false);
  const { resolvedTheme } = useTheme();
  const lineCount = value.split("\n").length;
  const lineHeightPx = 20;
  const computedHeight =
    minLines && maxLines
      ? `${Math.min(Math.max(lineCount, minLines), maxLines) * lineHeightPx}px`
      : height;

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(
    () => () => {
      if (copyResetTimerRef.current) clearTimeout(copyResetTimerRef.current);
    },
    [],
  );

  const copyValue = async () => {
    if (!navigator.clipboard) return;

    await navigator.clipboard.writeText(value);
    setCopied(true);
    if (copyResetTimerRef.current) clearTimeout(copyResetTimerRef.current);
    copyResetTimerRef.current = setTimeout(() => setCopied(false), 1500);
  };

  useEffect(() => {
    if (!containerRef.current) return;

    const useDarkTheme = isDarkTheme(resolvedTheme);

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        onChangeRef.current(update.state.doc.toString());
      }
    });
    const editorTheme = [
      useDarkTheme ? scryerDark : lightTheme,
      syntaxHighlighting(useDarkTheme ? oneDarkHighlightStyle : defaultHighlightStyle),
    ];
    const extensions = [
      lineNumbers(),
      ...languageExtensions(language),
      assistance.of([]),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      history(),
      appearance.of(editorTheme),
      editable.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
      updateListener,
      diagnosticField,
      diagnosticTheme,
      EditorView.lineWrapping,
      ...(language === "rego" ? [regoHighlightTheme, regoHighlightPlugin] : []),
    ];

    const state = EditorState.create({
      doc: value,
      extensions,
    });

    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;
    if (language !== "rego" && diagnostics.length > 0) {
      view.dispatch({
        effects: setDiagnosticsEffect.of(diagnostics),
      });
    }

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Only a language switch creates a new editor. Compartments preserve history.
    // Value and diagnostics are synchronized by dedicated effects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: [
      assistance.reconfigure(language === "rego" && regoFamily && regoTranslate
        ? regoAssistance(regoFamily, regoTranslate, readOnly)
        : scriptTrigger && scriptTranslate && ["shell", "python", "powershell", "batch", "go"].includes(language)
          ? scriptAssistance(language as ScriptEditorLanguage, scriptTrigger, scriptTranslate, readOnly)
          : []),
      appearance.reconfigure([
        isDarkTheme(resolvedTheme) ? scryerDark : lightTheme,
        syntaxHighlighting(isDarkTheme(resolvedTheme) ? oneDarkHighlightStyle : defaultHighlightStyle),
      ]),
      editable.reconfigure([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
    ] });
  }, [assistance, appearance, editable, language, regoFamily, regoTranslate, scriptTrigger, scriptTranslate, readOnly, resolvedTheme]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
      });
    }
  }, [value]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    if (language === "rego") {
      view.dispatch(setLintDiagnostics(view.state, readOnly ? [] : diagnostics.flatMap((diagnostic) => {
        if (diagnostic.line < 1 || diagnostic.line > view.state.doc.lines) return [];
        const line = view.state.doc.line(diagnostic.line);
        const from = Math.min(line.to, line.from + Math.max(0, (diagnostic.column ?? 1) - 1));
        return [{ from, to: Math.min(line.to, from + 1), severity: "error" as const, message: diagnostic.message ?? "" }];
      })));
    } else {
      view.dispatch({ effects: setDiagnosticsEffect.of(diagnostics) });
    }
  }, [diagnostics, language, readOnly]);

  useEffect(() => {
    if (!autoFocus) return;
    viewRef.current?.focus();
    onAutoFocus?.();
  }, [autoFocus, language, onAutoFocus, readOnly, resolvedTheme]);

  return (
    <div className="group/code-editor relative">
      {scriptTrigger && scriptTranslate && !readOnly ? (
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">{scriptTranslate("script.editor.assistanceHint")}</span>
          <Button type="button" variant="outline" size="sm" title={scriptTranslate("script.editor.formatHelp")} onClick={() => {
            if (viewRef.current) {
              formatScriptIndentation(viewRef.current);
              viewRef.current.focus();
            }
          }}>
            {scriptTranslate("script.editor.format")}
          </Button>
        </div>
      ) : null}
      <div
        id={id}
        ref={containerRef}
        className="overflow-hidden rounded-lg border border-border text-sm [&>.cm-editor]:h-full [&_.cm-scroller]:overflow-auto"
        style={{ height: computedHeight, minHeight: "120px" }}
      />
      {copyable ? (
        <IconButton
          label={copied ? copiedLabel : copyLabel}
          appearance="boxed"
          className="absolute right-2 top-2 z-10 bg-[var(--scry-surf)] opacity-0 shadow-sm transition-opacity group-hover/code-editor:opacity-100 group-focus-within/code-editor:opacity-100 focus-visible:opacity-100"
          onClick={() => void copyValue()}
        >
          {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
        </IconButton>
      ) : null}
    </div>
  );
}
