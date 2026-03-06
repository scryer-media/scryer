import { ExternalLink } from "lucide-react";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { getImdbUrl, getTvdbMovieUrl, formatRuntimeFromMinutes } from "./helpers";

export function InterstitialMoviePanel({ movie }: { movie: MetadataTvdbSearchItem }) {
  const imdbUrl = getImdbUrl(movie.imdbId);
  const tvdbUrl = getTvdbMovieUrl(movie);
  const runtime = formatRuntimeFromMinutes(movie.runtimeMinutes);

  return (
    <div className="flex items-start gap-4">
      <div className="shrink-0">
        {movie.posterUrl ? (
          <img
            src={movie.posterUrl}
            alt={movie.name}
            className="h-auto w-[140px] rounded-lg object-cover shadow-md"
          />
        ) : (
          <div className="flex h-[210px] w-[140px] items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60">
            No Poster
          </div>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-card-foreground">{movie.name}</p>
        <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
          {movie.year ? (
            <span>{movie.year}</span>
          ) : null}
          {runtime ? (
            <span>{runtime}</span>
          ) : null}
          {movie.status ? (
            <span className="capitalize">{movie.status}</span>
          ) : null}
        </div>
        {movie.overview ? (
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{movie.overview}</p>
        ) : (
          <p className="mt-3 text-sm italic text-muted-foreground/60">No description available.</p>
        )}
        <div className="mt-3 flex flex-wrap gap-2 text-sm">
          {imdbUrl ? (
            <a
              href={imdbUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-10 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-xs text-card-foreground hover:bg-muted"
              aria-label="Open on IMDb"
            >
              <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
              IMDb
            </a>
          ) : null}
          {tvdbUrl ? (
            <a
              href={tvdbUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-10 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-xs text-card-foreground hover:bg-muted"
              aria-label="Open on TVDB"
            >
              <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
              TVDB
            </a>
          ) : null}
        </div>
      </div>
    </div>
  );
}
