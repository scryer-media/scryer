import { lazy, Suspense } from "react";
import {
  createBrowserRouter,
  Navigate,
  Outlet,
  useLocation,
  useSearchParams,
} from "react-router";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { TranslateContext } from "@/lib/context/translate-context";
import { useLanguage } from "@/lib/hooks/use-language";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { resolveAppRoute } from "@/lib/utils/routing";
import { RouteErrorBoundary } from "./error-boundary";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));
const OAuthAuthorizePage = lazy(() => import("@/src/pages/oauth-authorize"));
const SetupPage = lazy(() => import("@/src/pages/setup"));

// This provider covers sibling routes such as login and setup. The selected
// dictionary is loaded before useLanguage commits a language change.
function RootTranslateBoundary() {
  const [searchParams] = useSearchParams();
  const { t } = useLanguage(searchParams);

  return (
    <TranslateContext.Provider value={t}>
      <Outlet />
    </TranslateContext.Provider>
  );
}

function ShellRoute() {
  const location = useLocation();
  const resolution = resolveAppRoute(
    location.pathname,
    location.search,
    location.hash,
  );

  // "landing" (`/`) deliberately falls through to the shell: the destination
  // depends on the signed-in user's permissions, which are not known until the
  // shell's auth bootstrap resolves.
  if (resolution.kind === "redirect") {
    return <Navigate to={resolution.to} replace />;
  }

  return (
    <Suspense fallback={<PageShellFallback />}>
      <RootPageShell />
    </Suspense>
  );
}

export const router = createBrowserRouter(
  [
    {
      element: <RootTranslateBoundary />,
      errorElement: <RouteErrorBoundary />,
      children: [
        {
          path: "/login",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <LoginPage />
            </Suspense>
          ),
        },
        {
          path: "/setup",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <SetupPage />
            </Suspense>
          ),
        },
        {
          path: "/oauth/authorize",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <OAuthAuthorizePage />
            </Suspense>
          ),
        },
        { path: "*", element: <ShellRoute /> },
      ],
    },
  ],
  { basename: getRuntimeBasePath() },
);
