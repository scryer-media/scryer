import { useTranslate } from "@/lib/context/translate-context";
import { Link } from "react-router";

export default function NotFound() {
  const t = useTranslate();
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-8">
      <div className="w-full max-w-lg rounded-lg border border-border bg-card/70 p-6 text-center">
        <h1 className="mb-2 text-xl font-semibold text-foreground">
          {t("uiText.pageNotFound")}
        </h1>
        <p className="mb-4 text-sm text-muted-foreground">
          {t("uiText.thePageYouRequestedDoesNotExist")}
        </p>
        <Link
          to="/"
          className="inline-flex h-10 items-center rounded-md border border-border bg-muted px-4 py-2 text-sm font-medium text-foreground transition-colors hover:bg-accent"
        >
          {t("uiText.goToHome")}
        </Link>
      </div>
    </main>
  );
}
