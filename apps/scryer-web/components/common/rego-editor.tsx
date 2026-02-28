import { useEffect, useRef } from "react";
import { EditorView, lineNumbers, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { oneDark } from "@codemirror/theme-one-dark";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { useTheme } from "next-themes";

type RegoEditorProps = {
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
};

const lightTheme = EditorView.theme({
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)" },
  ".cm-gutters": { backgroundColor: "var(--muted)", borderRight: "1px solid var(--border)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
});

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
      isDark ? oneDark : lightTheme,
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
      className="overflow-hidden rounded-md border border-border text-sm"
      style={{ height, minHeight: "120px" }}
    />
  );
}
