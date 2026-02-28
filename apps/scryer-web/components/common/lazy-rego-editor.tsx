import { lazy, Suspense } from "react";

const RegoEditor = lazy(() => import("./rego-editor"));

type LazyRegoEditorProps = {
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
};

function TextareaFallback({ value, onChange, readOnly, height = "320px" }: LazyRegoEditorProps) {
  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      readOnly={readOnly}
      className="w-full rounded-md border border-border bg-background p-3 font-mono text-sm text-foreground"
      style={{ height, minHeight: "120px", resize: "vertical" }}
    />
  );
}

export function LazyRegoEditor(props: LazyRegoEditorProps) {
  return (
    <Suspense fallback={<TextareaFallback {...props} />}>
      <RegoEditor {...props} />
    </Suspense>
  );
}
