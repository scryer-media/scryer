import { Rocket, ArrowRightLeft, Upload, type LucideIcon } from "lucide-react";

interface SetupWelcomeViewProps {
  t: (key: string) => string;
  onFreshSetup: () => void;
  onImportSetup: () => void;
  onRestoreSetup: () => void;
  onSkip: () => void;
  skipping: boolean;
  canRestoreSetup: boolean;
}

type WelcomeChoice = {
  id: string;
  icon: LucideIcon;
  title: string;
  description: string;
  onClick: () => void;
  tile: { background: string; borderColor: string; color: string };
};

const HEADING_FONT =
  "'Space Grotesk Variable', 'Inter Variable', ui-sans-serif, system-ui, sans-serif";

export function SetupWelcomeView({
  t,
  onFreshSetup,
  onImportSetup,
  onRestoreSetup,
  onSkip,
  skipping,
  canRestoreSetup,
}: SetupWelcomeViewProps) {
  const choices: WelcomeChoice[] = [
    {
      id: "setup-welcome-fresh",
      icon: Rocket,
      title: t("setup.freshSetup"),
      description: t("setup.freshSetupDescription"),
      onClick: onFreshSetup,
      tile: {
        background: "rgba(var(--scry-accent-rgb),0.14)",
        borderColor: "var(--scry-baccent)",
        color: "var(--scry-accent-text)",
      },
    },
    {
      id: "setup-welcome-import",
      icon: ArrowRightLeft,
      title: t("setup.importSetup"),
      description: t("setup.importSetupDescription"),
      onClick: onImportSetup,
      tile: {
        background: "rgba(92,200,245,0.14)",
        borderColor: "rgba(92,200,245,0.32)",
        color: "#5cc8f5",
      },
    },
    ...(canRestoreSetup
      ? [
          {
            id: "setup-welcome-restore",
            icon: Upload,
            title: t("setup.restoreSetup"),
            description: t("setup.restoreSetupDescription"),
            onClick: onRestoreSetup,
            tile: {
              background: "rgba(199,155,245,0.14)",
              borderColor: "rgba(199,155,245,0.32)",
              color: "#c79bf5",
            },
          } satisfies WelcomeChoice,
        ]
      : []),
  ];

  return (
    <div id="setup-welcome-view" className="flex flex-col items-center gap-8">
      <div className="text-center">
        <h1
          className="mb-3 text-3xl font-bold tracking-tight text-[var(--scry-ink2)]"
          style={{ fontFamily: HEADING_FONT }}
        >
          {t("setup.welcomeTitle")}
        </h1>
        <p className="text-[var(--scry-muted)]">
          {t("setup.welcomeDescription")}
        </p>
      </div>
      <div
        className={`grid w-full gap-4 ${
          canRestoreSetup
            ? "max-w-5xl xl:grid-cols-3"
            : "max-w-3xl md:grid-cols-2"
        }`}
      >
        {choices.map((choice) => {
          const Icon = choice.icon;
          return (
            <button
              key={choice.id}
              id={choice.id}
              type="button"
              onClick={choice.onClick}
              className="group flex flex-col items-center gap-3 rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surf)] p-6 text-center transition duration-200 [backdrop-filter:blur(12px)] hover:-translate-y-0.5 hover:border-[var(--scry-baccent)] hover:shadow-[0_18px_40px_rgba(2,6,23,0.18)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]"
            >
              <span
                className="flex h-12 w-12 items-center justify-center rounded-[14px] border transition duration-200 group-hover:scale-105"
                style={choice.tile}
              >
                <Icon className="h-6 w-6" />
              </span>
              <span>
                <span className="block text-[15px] font-semibold text-[var(--scry-ink2)]">
                  {choice.title}
                </span>
                <span className="mt-1 block text-[13px] leading-relaxed text-[var(--scry-muted)]">
                  {choice.description}
                </span>
              </span>
            </button>
          );
        })}
      </div>
      <button
        id="setup-welcome-skip"
        type="button"
        onClick={onSkip}
        disabled={skipping}
        className="text-sm text-[var(--scry-muted)] underline-offset-4 transition hover:text-[var(--scry-ink2)] hover:underline disabled:opacity-60"
      >
        {skipping ? t("setup.skipping") : t("setup.skipSetup")}
      </button>
    </div>
  );
}
