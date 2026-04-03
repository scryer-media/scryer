import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { FolderBrowserDialog } from "./folder-browser-dialog";

interface SetupMediaPathsViewProps {
  t: (key: string) => string;
  moviesPath: string;
  seriesPath: string;
  animePath: string;
  onMoviesPathChange: (value: string) => void;
  onSeriesPathChange: (value: string) => void;
  onAnimePathChange: (value: string) => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  saving: boolean;
  error: string | null;
}

type BrowseTarget = "movies" | "series" | "anime" | null;

export function SetupMediaPathsView({
  t,
  moviesPath,
  seriesPath,
  animePath,
  onMoviesPathChange,
  onSeriesPathChange,
  onAnimePathChange,
  onNext,
  onBack,
  onSkip,
  saving,
  error,
}: SetupMediaPathsViewProps) {
  const [browseTarget, setBrowseTarget] = useState<BrowseTarget>(null);
  const canProceed = moviesPath.trim().length > 0 && seriesPath.trim().length > 0;

  const browseInitialPath =
    browseTarget === "movies"
      ? moviesPath
      : browseTarget === "series"
        ? seriesPath
        : browseTarget === "anime"
          ? animePath
          : "/";

  function handleBrowseSelect(path: string) {
    if (browseTarget === "movies") onMoviesPathChange(path);
    else if (browseTarget === "series") onSeriesPathChange(path);
    else if (browseTarget === "anime") onAnimePathChange(path);
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.mediaPathsTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.mediaPathsDescription")}</p>
      </div>
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="movies-path">{t("setup.moviesPath")}</Label>
          <div className="flex gap-2">
            <Input
              id="movies-path"
              value={moviesPath}
              onChange={(e) => onMoviesPathChange(e.target.value)}
              placeholder="/data/movies"
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={() => setBrowseTarget("movies")}
              title={t("setup.browse")}
            >
              <FolderOpen className="h-4 w-4" />
            </Button>
          </div>
        </div>
        <div className="space-y-2">
          <Label htmlFor="series-path">{t("setup.seriesPath")}</Label>
          <div className="flex gap-2">
            <Input
              id="series-path"
              value={seriesPath}
              onChange={(e) => onSeriesPathChange(e.target.value)}
              placeholder="/data/series"
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={() => setBrowseTarget("series")}
              title={t("setup.browse")}
            >
              <FolderOpen className="h-4 w-4" />
            </Button>
          </div>
        </div>
        <div className="space-y-2">
          <Label htmlFor="anime-path">
            {t("setup.animePath")}
            <span className="ml-1.5 text-xs font-normal text-muted-foreground">
              {t("setup.optional")}
            </span>
          </Label>
          <div className="flex gap-2">
            <Input
              id="anime-path"
              value={animePath}
              onChange={(e) => onAnimePathChange(e.target.value)}
              placeholder="/data/anime"
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={() => setBrowseTarget("anime")}
              title={t("setup.browse")}
            >
              <FolderOpen className="h-4 w-4" />
            </Button>
          </div>
        </div>
        {error && <p className="text-sm text-destructive">{error}</p>}
      </div>
      <div className="flex items-center justify-between pt-2">
        <Button variant="ghost" onClick={onBack}>{t("setup.back")}</Button>
        <div className="flex items-center gap-3">
          {onSkip && (
            <button type="button" onClick={onSkip} className="text-sm text-muted-foreground underline-offset-4 hover:underline">
              {t("setup.skip")}
            </button>
          )}
          <Button onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </Button>
        </div>
      </div>

      <FolderBrowserDialog
        open={browseTarget !== null}
        onOpenChange={(open) => { if (!open) setBrowseTarget(null); }}
        onSelect={handleBrowseSelect}
        initialPath={browseInitialPath || "/"}
        title={t("setup.browse")}
      />
    </div>
  );
}
