import { useEffect } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { SetupWizardContainer } from "@/components/setup/setup-wizard-container";

export default function SetupPage() {
  const { user, loading: authLoading } = useAuth();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();

  useEffect(() => {
    if (!authLoading && !user) {
      navigate("/login", { replace: true });
    }
  }, [authLoading, user, navigate]);

  const { uiLanguage, t } = useLanguage(searchParams);

  const isReentry = searchParams.get("reentry") === "1";

  if (authLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background text-foreground">
        <Loader2 className="h-6 w-6 animate-spin text-emerald-700 dark:text-emerald-300" />
      </div>
    );
  }

  if (!user) return null;

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <div className="min-h-screen bg-background text-foreground">
        <SetupWizardContainer t={t} isReentry={isReentry} />
      </div>
    </ScryerGraphqlProvider>
  );
}
