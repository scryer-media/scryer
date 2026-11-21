import * as React from "react";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";
import type { LifecycleTestSubject } from "@/lib/types/lifecycle-rule-sets";

/** Choose a title to preview; preview resolves its ownership again. */
export function LifecycleSubjectPicker({
  onChange,
}: {
  onChange: (subject: LifecycleTestSubject | undefined) => void;
}) {
  const t = useTranslate();
  const [title, setTitle] = React.useState<TitleRecord | null>(null);
  return (
    <div className="grid gap-3 md:grid-cols-3">
      <TitleAutocompletePicker
        selectedTitle={title}
        selectedTitleId={title?.id ?? null}
        ariaLabel={t("settings.lifecycleTestTitle")}
        placeholder={t("settings.lifecycleTestTitle")}
        onSelectedTitleChange={(next) => {
          setTitle(next);
          onChange(next ? { titleId: next.id, subjectId: next.id } : undefined);
        }}
      />
    </div>
  );
}
