import { useEffect } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { SetupWizardContainer } from "@/components/setup/setup-wizard-container";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";

export default function SetupPage() {
  const { serviceRestarting } = useBackendRestarting();
  const { user, loading: authLoading } = useAuth();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();

  useEffect(() => {
    if (!serviceRestarting && !authLoading && !user) {
      navigate("/login", { replace: true });
    }
  }, [authLoading, user, navigate, serviceRestarting]);

  const { uiLanguage, t } = useLanguage(searchParams);

  const isReentry = searchParams.get("reentry") === "1";

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

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
