import { useMemo, type ReactNode } from "react";
import {
  LazyCodeEditor,
  type CodeEditorDiagnostic,
  type CodeEditorProps,
} from "@/components/common/lazy-code-editor";

import { useTranslate } from "@/lib/context/translate-context";
import { useRuleOutputDescriptors } from "@/lib/hooks/use-rule-builder-schema";
import {
  REGO_FAMILY_OUTPUTS,
  regoHeadAssists,
  type RegoFamily,
} from "@/lib/utils/rego-assistance";

export type RegoEditorDiagnostic = CodeEditorDiagnostic;

export type LazyRegoEditorProps = Omit<
  CodeEditorProps,
  "language" | "regoFamily" | "regoTranslate" | "regoHeads"
> & {
  ruleFamily: RegoFamily;
  /// The rule's variant (a maintenance rule's scope), which narrows the heads
  /// offered. Null or absent offers every head.
  ruleVariant?: string | null;
  feedback?: ReactNode;
};

export function LazyRegoEditor({
  ruleFamily,
  ruleVariant,
  feedback,
  ...props
}: LazyRegoEditorProps) {
  const t = useTranslate();
  const descriptors = useRuleOutputDescriptors(REGO_FAMILY_OUTPUTS[ruleFamily]);
  const heads = useMemo(
    () => regoHeadAssists(descriptors, t, ruleVariant),
    [descriptors, t, ruleVariant],
  );
  return (
    <div className="relative">
      <LazyCodeEditor
        {...props}
        language="rego"
        regoFamily={ruleFamily}
        regoTranslate={t}
        regoHeads={heads}
      />
      {feedback}
    </div>
  );
}
