import { useEffect, useRef } from "react";
import { EditorView, lineNumbers, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { syntaxHighlighting } from "@codemirror/language";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { useTheme } from "next-themes";

type RegoEditorProps = {
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
};

const CODE_FONT = "'Fira Code', 'Fira Mono', 'JetBrains Mono', 'Source Code Pro', 'Cascadia Code', 'Consolas', monospace";

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

export default function RegoEditor({ value, onChange, readOnly = false, height = "320px" }: RegoEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const { resolvedTheme } = useTheme();

  onChangeRef.current = onChange;

  useEffect(() => {
    if (!containerRef.current) return;

    const isDark = resolvedTheme === "dark";

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        onChangeRef.current(update.state.doc.toString());
      }
    });

    const extensions = [
      lineNumbers(),
      javascript(),
      keymap.of([...defaultKeymap, indentWithTab]),
      updateListener,
      EditorView.lineWrapping,
      isDark ? [scryerDark, syntaxHighlighting(oneDarkHighlightStyle)] : lightTheme,
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

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Recreate editor when theme changes. Value is set once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedTheme, readOnly]);

  // Sync external value changes (e.g. loading a rule for edit)
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

  return (
    <div
      ref={containerRef}
      className="overflow-hidden rounded-lg border border-border text-sm"
      style={{ height, minHeight: "120px" }}
    />
  );
}
