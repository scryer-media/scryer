import { useEffect, useState } from "react";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { TITLE_MEDIA_FILE_FIELDS } from "@/lib/graphql/queries";
import type { MediaAnalysisDetails } from "@/lib/types/media-analysis";
import type { TitleMediaFileRecord as TitleMediaFile } from "@/lib/types/titles";

const selectDiscTitleMutation = `
  mutation SelectMediaFileDiscTitle($fileId: ID!, $discTitleId: String) {
    selectMediaFileDiscTitle(fileId: $fileId, discTitleId: $discTitleId) {
      ${TITLE_MEDIA_FILE_FIELDS}
    }
  }
`;

export function DiscTitleSelection({ fileId, analysis, onChanged }: {
  fileId: string;
  analysis: MediaAnalysisDetails;
  onChanged: (analysis: MediaAnalysisDetails) => void;
}) {
  const client = useClient();
  const t = useTranslate();
  const savedIdentity = analysis.disc?.selection.titleId ?? "";
  const savedTitle = analysis.disc?.titles.find((title) => title.id === savedIdentity || title.aliases.includes(savedIdentity))?.id ?? savedIdentity;
  const [selected, setSelected] = useState(savedTitle);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setSelected(savedTitle); }, [savedTitle]);
  if (!analysis.disc) return null;

  async function save() {
    setSaving(true);
    setError(null);
    try {
      const result = await client.mutation<{ selectMediaFileDiscTitle: TitleMediaFile }>(
        selectDiscTitleMutation, { fileId, discTitleId: selected || null },
      ).toPromise();
      if (result.error) throw result.error;
      const next = result.data?.selectMediaFileDiscTitle.analysis;
      if (!next) throw new Error(t("mediaFile.discSelectionFailed"));
      onChanged(next);
    } catch (error) {
      setError(error instanceof Error ? error.message : t("mediaFile.discSelectionFailed"));
    } finally {
      setSaving(false);
    }
  }

  return <div className="my-2 space-y-2">
    <label className="flex flex-col gap-1">
      {t("mediaFile.discTitle")}
      <select className="rounded border bg-background p-2" value={selected} disabled={saving}
        onChange={(event) => setSelected(event.target.value)}>
        <option value="">{t("mediaFile.discAutomatic")}</option>
        {savedTitle && !analysis.disc.titles.some((title) => title.id === savedTitle || title.aliases.includes(savedTitle))
          ? <option value={savedTitle} disabled>{savedTitle}</option> : null}
        {analysis.disc.titles.map((title) => <option key={title.id} value={title.id}>
          {title.id} · {title.durationSeconds == null ? "?" : `${(title.durationSeconds / 60).toFixed(1)} min`} · {title.report.status.toLowerCase()}
        </option>)}
      </select>
    </label>
    <button type="button" className="rounded border px-2 py-1 disabled:opacity-50"
      disabled={saving || selected === savedTitle} onClick={() => { void save(); }}>
      {t(saving ? "mediaFile.discSaving" : "mediaFile.discSave")}
    </button>
    {error ? <p role="alert" className="text-destructive">{error}</p> : null}
  </div>;
}
