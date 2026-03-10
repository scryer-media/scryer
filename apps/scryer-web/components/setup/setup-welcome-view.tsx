import { Rocket, ArrowRightLeft } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";

interface SetupWelcomeViewProps {
  t: (key: string) => string;
  onFreshSetup: () => void;
  onImportSetup: () => void;
  onSkip: () => void;
  skipping: boolean;
}

export function SetupWelcomeView({ t, onFreshSetup, onImportSetup, onSkip, skipping }: SetupWelcomeViewProps) {
  return (
    <div className="flex flex-col items-center gap-8">
      <div className="text-center">
        <h1
          className="mb-3 text-3xl font-bold tracking-tight"
          style={{ fontFamily: "'Space Grotesk', Inter, ui-sans-serif, system-ui, sans-serif" }}
        >
          {t("setup.welcomeTitle")}
        </h1>
        <p className="text-muted-foreground">{t("setup.welcomeDescription")}</p>
      </div>
      <div className="grid w-full max-w-2xl gap-4 md:grid-cols-2">
        <Card
          className="cursor-pointer transition-colors hover:border-primary"
          onClick={onFreshSetup}
        >
          <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
            <Rocket className="h-8 w-8 text-emerald-500" />
            <div>
              <p className="font-semibold">{t("setup.freshSetup")}</p>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("setup.freshSetupDescription")}
              </p>
            </div>
          </CardContent>
        </Card>
        <Card
          className="cursor-pointer transition-colors hover:border-primary"
          onClick={onImportSetup}
        >
          <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
            <ArrowRightLeft className="h-8 w-8 text-blue-500" />
            <div>
              <p className="font-semibold">{t("setup.importSetup")}</p>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("setup.importSetupDescription")}
              </p>
            </div>
          </CardContent>
        </Card>
      </div>
      <button
        type="button"
        onClick={onSkip}
        disabled={skipping}
        className="text-sm text-muted-foreground underline-offset-4 hover:underline"
      >
        {skipping ? t("setup.skipping") : t("setup.skipSetup")}
      </button>
    </div>
  );
}
