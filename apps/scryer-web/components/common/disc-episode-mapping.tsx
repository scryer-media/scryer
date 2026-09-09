import { useEffect, useState } from "react";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { TITLE_MEDIA_FILE_FIELDS } from "@/lib/graphql/queries";
import type { MediaAnalysisDetails, MediaDiscMetadata } from "@/lib/types/media-analysis";
import { discEpisodeSelections } from "@/lib/utils/disc-review";
import type { TitleMediaFileRecord } from "@/lib/types/titles";

type Target = { episodeId: string; label: string; durationSeconds: number | string | null };
const targetsMutation = `mutation DiscEpisodeTargets($fileId: ID!) {
  mediaFileDiscEpisodeTargets(fileId: $fileId) { episodeId label durationSeconds }
}`;
const mappingMutation = `mutation MapDiscEpisodes($fileId: ID!, $mappings: [MediaDiscEpisodeMappingInput!]!) {
  mapMediaFileDiscEpisodes(fileId: $fileId, mappings: $mappings) { ${TITLE_MEDIA_FILE_FIELDS} }
}`;

export function DiscEpisodeMapping({ fileId, analysis, onChanged, inventory = analysis.disc }: {
  fileId: string; analysis: MediaAnalysisDetails; onChanged: (analysis: MediaAnalysisDetails) => void;
  inventory?: MediaDiscMetadata | null;
}) {
  const client = useClient();
  const t = useTranslate();
  const [targets, setTargets] = useState<Target[] | null>(null);
  const [mappings, setMappings] = useState<Record<string, string>>(() => discEpisodeSelections(analysis.disc, inventory));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setMappings(discEpisodeSelections(analysis.disc, inventory)); }, [analysis.disc, inventory]);
  async function load() {
    setBusy(true); setError(null);
    try {
      const result = await client.mutation<{ mediaFileDiscEpisodeTargets: Target[] }>(targetsMutation, { fileId }).toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("mediaFile.discMappingFailed"));
      setTargets(result.data.mediaFileDiscEpisodeTargets);
    } catch (error) { setError(error instanceof Error ? error.message : t("mediaFile.discMappingFailed")); }
    finally { setBusy(false); }
  }
  async function save() {
    setBusy(true); setError(null);
    try {
      const result = await client.mutation<{ mapMediaFileDiscEpisodes: TitleMediaFileRecord }>(mappingMutation, {
        fileId, mappings: Object.entries(mappings).filter(([, episodeId]) => episodeId).map(([discTitleId, episodeId]) => ({ discTitleId, episodeId })),
      }, { additionalTypenames: ["TitlePayload", "EpisodePayload", "CollectionPayload", "EpisodeMediaAvailabilityPayload"] }).toPromise();
      if (result.error) throw result.error;
      const next = result.data?.mapMediaFileDiscEpisodes.analysis;
      if (!next) throw new Error(t("mediaFile.discMappingFailed"));
      onChanged(next);
    } catch (error) { setError(error instanceof Error ? error.message : t("mediaFile.discMappingFailed")); }
    finally { setBusy(false); }
  }
  const titleIds = [...new Set([...(inventory?.titles.map((title) => title.id) ?? []), ...Object.keys(mappings)])];
  return <div className="my-2 space-y-2">
    {targets == null ? <button type="button" disabled={busy} className="rounded border px-2 py-1 disabled:opacity-50"
      onClick={() => { void load(); }}>{t(busy ? "mediaFile.discMappingLoading" : "mediaFile.discEpisodeMapping")}</button>
      : targets.length === 0 ? <p>{t("mediaFile.discMappingEmpty")}</p>
      : <>
        <p className="text-muted-foreground">{t("mediaFile.discMappingScope")}</p>
        {titleIds.map((id) => {
          const title = inventory?.titles.find((item) => item.id === id);
          const selected = mappings[id] ?? "";
          return <label key={id} className="flex items-center gap-2">
            <span>{id} · {title?.durationSeconds == null ? "?" : `${(title.durationSeconds / 60).toFixed(1)} min`}</span>
            <select className="min-w-0 flex-1 rounded border bg-background p-1" value={selected} disabled={busy}
              onChange={(event) => setMappings((previous) => ({ ...previous, [id]: event.target.value }))}>
              <option value="">{t("mediaFile.discMappingNone")}</option>
              {selected && !targets.some((target) => target.episodeId === selected) ? <option value={selected} disabled>{selected}</option> : null}
              {targets.map((target) => <option key={target.episodeId} value={target.episodeId}
                disabled={Object.entries(mappings).some(([key, value]) => key !== id && value === target.episodeId)}>{target.label}</option>)}
            </select>
          </label>;
        })}
        <button type="button" disabled={busy} className="rounded border px-2 py-1 disabled:opacity-50"
          onClick={() => { void save(); }}>{t(busy ? "mediaFile.discSaving" : "mediaFile.discMappingSave")}</button>
      </>}
    {error ? <p role="alert" className="text-destructive">{error}</p> : null}
  </div>;
}
