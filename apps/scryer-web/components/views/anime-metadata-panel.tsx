import * as React from "react";
import { Star, Tv, Radio } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  MAL_SCORE_PREFIX,
  ANIME_MEDIA_TYPE_PREFIX,
  ANIME_STATUS_PREFIX,
  getTagValue,
} from "@/lib/utils/title-tags";
import { useTranslate } from "@/lib/context/translate-context";
import type { CollectionEpisode } from "@/components/containers/series-overview-container";

type AnimeMetadataPanelProps = {
  tags: string[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
};

function formatAnimeStatus(raw: string): string {
  const normalized = raw.toLowerCase().replace(/[_\s]+/g, " ").trim();
  if (normalized.includes("airing") && !normalized.includes("not")) return "Currently Airing";
  if (normalized.includes("finished")) return "Finished Airing";
  if (normalized.includes("not yet")) return "Not Yet Aired";
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

function formatAnimeMediaType(raw: string): string {
  const map: Record<string, string> = {
    tv: "TV",
    ova: "OVA",
    movie: "Movie",
    special: "Special",
    ona: "ONA",
    music: "Music",
  };
  return map[raw.toLowerCase()] ?? raw;
}

export function AnimeMetadataPanel({ tags, episodesByCollection }: AnimeMetadataPanelProps) {
  const t = useTranslate();
  const malScore = getTagValue(tags, MAL_SCORE_PREFIX);
  const animeMediaType = getTagValue(tags, ANIME_MEDIA_TYPE_PREFIX);
  const animeStatus = getTagValue(tags, ANIME_STATUS_PREFIX);
  const { fillerCount, totalCount } = React.useMemo(() => {
    let filler = 0;
    let total = 0;
    for (const episodes of Object.values(episodesByCollection)) {
      for (const ep of episodes) {
        total++;
        if (ep.isFiller) filler++;
      }
    }
    return { fillerCount: filler, totalCount: total };
  }, [episodesByCollection]);

  const fillerPercent = totalCount > 0 ? Math.round((fillerCount / totalCount) * 100) : 0;
  const hasAnyData = malScore || animeMediaType || animeStatus || fillerCount > 0;
  if (!hasAnyData) return null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Tv className="h-4 w-4" />
          {t("anime.metadataPanel")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex flex-wrap gap-x-6 gap-y-3">
            {malScore ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{t("anime.malScore")}</p>
                <p className="flex items-center gap-1 text-sm font-semibold text-foreground">
                  <Star className="h-3.5 w-3.5 text-amber-500" />
                  {malScore}
                </p>
              </div>
            ) : null}
            {animeMediaType ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{t("anime.mediaType")}</p>
                <p className="text-sm font-semibold text-foreground">
                  {formatAnimeMediaType(animeMediaType)}
                </p>
              </div>
            ) : null}
            {animeStatus ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{t("anime.status")}</p>
                <p className="flex items-center gap-1 text-sm font-semibold text-foreground">
                  <Radio className="h-3.5 w-3.5" />
                  {formatAnimeStatus(animeStatus)}
                </p>
              </div>
            ) : null}
            {totalCount > 0 ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{t("anime.fillerCount", { count: fillerCount })}</p>
                <p className="text-sm text-foreground">
                  {fillerCount > 0
                    ? t("anime.fillerSummary", {
                        fillerCount: String(fillerCount),
                        totalCount: String(totalCount),
                        percent: String(fillerPercent),
                      })
                    : t("anime.noFiller")}
                </p>
              </div>
            ) : null}
        </div>
      </CardContent>
    </Card>
  );
}
