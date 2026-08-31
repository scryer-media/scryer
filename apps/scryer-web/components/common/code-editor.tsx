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
  EditorState,
  StateEffect,
  StateField,
  type Extension,
  type Range,
} from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/legacy-modes/mode/javascript";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { xml } from "@codemirror/legacy-modes/mode/xml";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { useTheme } from "next-themes";
import { Check, Copy } from "lucide-react";
import "@fontsource-variable/jetbrains-mono";
import { IconButton } from "@/components/ui/icon-button";
import { CODE_FONT } from "@/lib/fonts";
import { isDarkTheme } from "@/lib/theme";

export type CodeEditorLanguage = "plain" | "javascript" | "json" | "rego" | "shell" | "xml";

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
  copyable?: boolean;
  copyLabel?: string;
  copiedLabel?: string;
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
  ".cm-gutters": { backgroundColor: "var(--muted)", borderRight: "1px solid var(--border)", paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-content": { fontFamily: CODE_FONT, paddingLeft: "8px" },
  ".cm-gutters .cm-gutter": { fontFamily: CODE_FONT },
});

const scryerDark = EditorView.theme({
  "&": { backgroundColor: "#0a0e1a", color: "#d4d4d8" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "#5b64ff", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#5b64ff" },
  ".cm-gutters": { backgroundColor: "#0a0e1a", color: "#3f3f46", borderRight: "1px solid #273255", fontFamily: CODE_FONT, paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "rgba(255,255,255,0.03)", color: "#71717a" },
  ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.03)" },
  "&.cm-focused": { outline: "2px solid hsl(var(--ring))" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(91,100,255,0.2)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(91,100,255,0.3)" },
  ".cm-line": { padding: "0 4px" },
}, { dark: true });

const prideTheme = EditorView.theme({
  "&": { backgroundColor: "#100d20", color: "#fff0fb" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "#ff5ca8", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#ff5ca8" },
  ".cm-gutters": { backgroundColor: "#0d0a1b", color: "#9587b7", borderRight: "1px solid rgba(255,255,255,0.14)", fontFamily: CODE_FONT, paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "rgba(255,255,255,0.05)", color: "#ffd8f0" },
  ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.04)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(255,92,168,0.24)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(255,92,168,0.35)" },
  ".cm-line": { padding: "0 4px" },
}, { dark: true });

function languageExtensions(language: CodeEditorLanguage): Extension[] {
  if (language === "javascript") {
    return [javascript()];
  }

  if (language === "json") {
    return [StreamLanguage.define(json)];
  }

  if (language === "shell") {
    return [StreamLanguage.define(shell)];
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
  copyable = false,
  copyLabel = "Copy code",
  copiedLabel = "Copied",
}: CodeEditorProps) {
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

    const usePrideTheme = resolvedTheme === "pride";
    const useDarkTheme = isDarkTheme(resolvedTheme);

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        onChangeRef.current(update.state.doc.toString());
      }
    });
    const editorTheme = usePrideTheme ? prideTheme : useDarkTheme ? scryerDark : lightTheme;
    const extensions = [
      lineNumbers(),
      ...languageExtensions(language),
      syntaxHighlighting(oneDarkHighlightStyle),
      keymap.of([...defaultKeymap, indentWithTab]),
      updateListener,
      diagnosticField,
      diagnosticTheme,
      EditorView.lineWrapping,
      editorTheme,
      ...(language === "rego" ? [regoHighlightTheme, regoHighlightPlugin] : []),
    ];

    if (readOnly) {
      extensions.push(EditorState.readOnly.of(true));
    }

    const state = EditorState.create({
      doc: value,
      extensions,
    });

    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;
    if (diagnostics.length > 0) {
      view.dispatch({
        effects: setDiagnosticsEffect.of(diagnostics),
      });
    }

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Recreate editor when theme, language, or read-only behavior changes.
    // Value and diagnostics are synchronized by dedicated effects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedTheme, readOnly, language]);

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
    view.dispatch({
      effects: setDiagnosticsEffect.of(diagnostics),
    });
  }, [diagnostics]);

  return (
    <div className="group/code-editor relative">
      <div
        id={id}
        ref={containerRef}
        className="overflow-auto rounded-lg border border-border text-sm"
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
