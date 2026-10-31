import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { LoadingMark } from "@/components/common/loading-mark";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { movieAvailabilitySettingsQuery } from "@/lib/graphql/queries";
import { updateMovieAvailabilitySettingsMutation } from "@/lib/graphql/mutations";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAnyAppPermission } from "@/lib/utils/permissions";
import {
  MOVIE_AVAILABILITY_VALUES,
  type MovieAvailabilitySettings,
} from "@/lib/types/settings";

type MovieAvailabilitySettingsResult = {
  mediaSettings?: MovieAvailabilitySettings | null;
};

type UpdateMovieAvailabilitySettingsResult = {
  updateMediaSettings?: MovieAvailabilitySettings | null;
};

/**
 * The Movie facet's availability gate: the threshold for movies without their
 * own, the release market whose dates are read first, and the day offset.
 * Movie libraries can override each one.
 */
export function MovieAvailabilitySettingsFields() {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const { user } = useAuth();
  const canManage = hasAnyAppPermission(user, [
    APP_PERMISSIONS.manageCatalogSettings,
  ]);
  const [draft, setDraft] = useState<MovieAvailabilitySettings | null>(null);
  const [saving, setSaving] = useState(false);

  const fetchSettings = useCallback(async () => {
    const { data, error } = await client
      .query<MovieAvailabilitySettingsResult>(
        movieAvailabilitySettingsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise();
    if (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
      return;
    }
    setDraft(data?.mediaSettings ?? null);
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void fetchSettings();
  }, [fetchSettings]);

  if (!draft) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoadingMark className="h-4 w-4" />
        {t("label.loading")}
      </div>
    );
  }

  const disabled = !canManage || saving;
  const update = (patch: Partial<MovieAvailabilitySettings>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  const save = async () => {
    setSaving(true);
    try {
      const { data, error } = await client
        .mutation<UpdateMovieAvailabilitySettingsResult>(
          updateMovieAvailabilitySettingsMutation,
          {
            input: {
              scope: "MOVIE",
              defaultMovieAvailability: draft.defaultMovieAvailability,
              movieReleaseMarket: draft.movieReleaseMarket,
              movieAvailabilityDelayDays: draft.movieAvailabilityDelayDays,
            },
          },
        )
        .toPromise();
      if (error) throw error;
      if (data?.updateMediaSettings) setDraft(data.updateMediaSettings);
      setGlobalStatus(t("settings.mediaSettingsSaved"), { level: "SUCCESS" });
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        {
          level: "ERROR",
        },
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div id="settings-movie-availability" className="space-y-5">
      <div className="grid gap-3 md:grid-cols-3">
        <div className="space-y-2">
          <Label htmlFor="settings-movie-default-availability">
            {t("settings.movieDefaultAvailability")}
          </Label>
          <Select
            value={draft.defaultMovieAvailability ?? "released"}
            disabled={disabled}
            onValueChange={(defaultMovieAvailability) =>
              update({ defaultMovieAvailability })
            }
          >
            <SelectTrigger
              id="settings-movie-default-availability"
              className="w-full"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {MOVIE_AVAILABILITY_VALUES.map((value) => (
                <SelectItem key={value} value={value}>
                  {t(`settings.minAvailability.${value}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            {t("settings.movieDefaultAvailabilityHelp")}
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor="settings-movie-release-market">
            {t("settings.movieReleaseMarket")}
          </Label>
          <Input
            id="settings-movie-release-market"
            value={draft.movieReleaseMarket ?? ""}
            maxLength={2}
            disabled={disabled}
            onChange={(event) =>
              update({ movieReleaseMarket: event.target.value.toUpperCase() })
            }
            className="max-w-[120px]"
          />
          <p className="text-xs text-muted-foreground">
            {t("settings.movieReleaseMarketHelp")}
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor="settings-movie-availability-delay-days">
            {t("settings.movieAvailabilityDelay")}
          </Label>
          <Input
            id="settings-movie-availability-delay-days"
            type="number"
            value={draft.movieAvailabilityDelayDays ?? 0}
            disabled={disabled}
            onChange={(event) => {
              const next = Number.parseInt(event.target.value, 10);
              update({
                movieAvailabilityDelayDays: Number.isNaN(next) ? 0 : next,
              });
            }}
            className="max-w-[160px]"
          />
          <p className="text-xs text-muted-foreground">
            {t("settings.movieAvailabilityDelayHelp")}
          </p>
        </div>
      </div>
      <Button
        id="settings-movie-availability-save"
        disabled={disabled}
        onClick={() => void save()}
      >
        {saving ? <LoadingMark className="mr-1 h-4 w-4" /> : null}
        {t("label.save")}
      </Button>
    </div>
  );
}
