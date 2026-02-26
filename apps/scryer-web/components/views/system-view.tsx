
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type SystemViewState = {
  t: Translate;
  systemHealth: SystemHealth | null;
  systemLoading: boolean;
  refreshSystem: () => Promise<void>;
};

type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesTv: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
};

export function SystemView({
  state,
}: {
  state: SystemViewState;
}) {
  const { t, systemHealth, systemLoading, refreshSystem } = state;

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("system.title")}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="mb-3 flex gap-2">
          <Button size="sm" variant="secondary" onClick={() => void refreshSystem()} disabled={systemLoading}>
            {systemLoading ? t("system.refreshing") : t("label.refresh")}
          </Button>
        </div>
        {!systemHealth ? (
          <p className="text-sm text-muted-foreground">{t("system.notLoaded")}</p>
        ) : (
          <div className="space-y-2">
            <p className="text-sm">
              {t("system.serviceReady")}: {systemHealth.serviceReady ? t("label.yes") : t("label.no")}
            </p>
            <p className="text-sm">
              {t("system.dbPathLabel")}: {systemHealth.dbPath}
            </p>
            <p className="text-sm">
              {t("system.totalTitlesLabel")}: {systemHealth.totalTitles}
            </p>
            <p className="text-sm">
              {t("system.monitoredTitlesLabel")}: {systemHealth.monitoredTitles}
            </p>
            <p className="text-sm">
              {t("system.usersLabel")}: {systemHealth.totalUsers}
            </p>
            <p className="text-sm">
              {t("system.facetLabel")}: movie={systemHealth.titlesMovie}, tv={systemHealth.titlesTv}, anime=
              {systemHealth.titlesAnime}, other={systemHealth.titlesOther}
            </p>
            <Separator />
            <p className="text-sm">{t("system.recentEventsLabel")}</p>
            <ul className="space-y-1 text-sm text-card-foreground">
              {systemHealth.recentEventPreview.map((entry) => (
                <li key={entry} className="rounded-xl border border-border bg-card p-2">
                  {entry}
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
