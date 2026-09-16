import { useEffect } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { SetupWizardContainer } from "@/components/setup/setup-wizard-container";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { SetupIntroLoading } from "@/components/setup/setup-intro";

export default function SetupPage() {
  const { serviceRestarting, setServiceRestarting } = useBackendRestarting();
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
    return <SetupIntroLoading />;
  }

  if (!user) return null;

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <div className="min-h-screen bg-fixed text-[var(--scry-body)] [background-image:var(--scry-shell-bg)]">
        <SetupWizardContainer
          t={t}
          isReentry={isReentry}
          onBackendRestarting={() => setServiceRestarting(true)}
        />
      </div>
    </ScryerGraphqlProvider>
  );
}
