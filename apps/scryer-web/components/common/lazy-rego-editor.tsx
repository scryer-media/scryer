import type { ReactNode } from "react";
import {
  LazyCodeEditor,
  type CodeEditorDiagnostic,
  type CodeEditorProps,
} from "@/components/common/lazy-code-editor";

import { useTranslate } from "@/lib/context/translate-context";
import type { RegoFamily } from "@/lib/utils/rego-assistance";

export type RegoEditorDiagnostic = CodeEditorDiagnostic;

export type LazyRegoEditorProps = Omit<
  CodeEditorProps,
  "language" | "regoFamily" | "regoTranslate"
> & { ruleFamily: RegoFamily; feedback?: ReactNode };

export function LazyRegoEditor({
  ruleFamily,
  feedback,
  ...props
}: LazyRegoEditorProps) {
  const t = useTranslate();
  return (
    <div className="relative">
      <LazyCodeEditor
        {...props}
        language="rego"
        regoFamily={ruleFamily}
        regoTranslate={t}
      />
      {feedback}
    </div>
  );
}
