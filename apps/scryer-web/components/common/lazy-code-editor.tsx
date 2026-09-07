import { lazy, Suspense } from "react";
import { cn } from "@/lib/utils";
import type {
  CodeEditorDiagnostic,
  CodeEditorLanguage,
  CodeEditorProps,
} from "@/components/common/code-editor";

const CodeEditor = lazy(() => import("./code-editor"));

export type {
  CodeEditorDiagnostic,
  CodeEditorLanguage,
  CodeEditorProps,
};

function TextareaFallback({
  id,
  value,
  onChange,
  readOnly,
  autoFocus,
  height = "320px",
  minLines,
  maxLines,
}: CodeEditorProps) {
  const lineCount = value.split("\n").length;
  const lineHeightPx = 20;
  const autoHeight =
    minLines && maxLines
      ? `${Math.min(Math.max(lineCount, minLines), maxLines) * lineHeightPx}px`
      : height;

  return (
    <textarea
      id={id}
      autoFocus={autoFocus}
      autoComplete="off"
      data-1p-ignore="true"
      data-lpignore="true"
      data-bwignore="true"
      data-form-type="other"
      data-protonpass-ignore="true"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      readOnly={readOnly}
      className={cn(
        "w-full rounded-lg border border-border bg-background p-3 font-[var(--font-code)] text-sm text-foreground",
        "disabled:cursor-not-allowed disabled:opacity-60",
      )}
      style={{ height: autoHeight, minHeight: "120px", resize: "vertical" }}
    />
  );
}

export function LazyCodeEditor(props: CodeEditorProps) {
  return (
    <Suspense fallback={<TextareaFallback {...props} />}>
      <CodeEditor {...props} />
    </Suspense>
  );
}
