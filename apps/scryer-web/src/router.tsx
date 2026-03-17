import { lazy, Suspense } from "react";
import { createBrowserRouter } from "react-router-dom";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { RouteErrorBoundary } from "./error-boundary";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));
const SetupPage = lazy(() => import("@/src/pages/setup"));

function ShellRoute() {
  return (
    <Suspense fallback={<PageShellFallback />}>
      <RootPageShell />
    </Suspense>
  );
}

export const router = createBrowserRouter(
  [
    {
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
        { path: "/", element: <ShellRoute /> },
        { path: "/movies", element: <ShellRoute /> },
        { path: "/movies/overview", element: <ShellRoute /> },
        { path: "/movies/settings", element: <ShellRoute /> },
        { path: "/movies/settings/general", element: <ShellRoute /> },
        { path: "/movies/settings/quality", element: <ShellRoute /> },
        { path: "/movies/settings/renaming", element: <ShellRoute /> },
        { path: "/movies/settings/routing", element: <ShellRoute /> },
        { path: "/series", element: <ShellRoute /> },
        { path: "/series/overview", element: <ShellRoute /> },
        { path: "/series/settings", element: <ShellRoute /> },
        { path: "/series/settings/general", element: <ShellRoute /> },
        { path: "/series/settings/quality", element: <ShellRoute /> },
        { path: "/series/settings/renaming", element: <ShellRoute /> },
        { path: "/series/settings/routing", element: <ShellRoute /> },
        { path: "/anime", element: <ShellRoute /> },
        { path: "/anime/overview", element: <ShellRoute /> },
        { path: "/anime/settings", element: <ShellRoute /> },
        { path: "/anime/settings/general", element: <ShellRoute /> },
        { path: "/anime/settings/quality", element: <ShellRoute /> },
        { path: "/anime/settings/renaming", element: <ShellRoute /> },
        { path: "/anime/settings/routing", element: <ShellRoute /> },
        { path: "/activity", element: <ShellRoute /> },
        { path: "/wanted", element: <ShellRoute /> },
        { path: "/settings", element: <ShellRoute /> },
        { path: "/settings/profile", element: <ShellRoute /> },
        { path: "/settings/indexers", element: <ShellRoute /> },
        { path: "/settings/download-clients", element: <ShellRoute /> },
        { path: "/settings/quality-profiles", element: <ShellRoute /> },
        { path: "/settings/delay-profiles", element: <ShellRoute /> },
        { path: "/settings/general", element: <ShellRoute /> },
        { path: "/settings/users", element: <ShellRoute /> },
        { path: "/settings/acquisition", element: <ShellRoute /> },
        { path: "/settings/post-processing", element: <ShellRoute /> },
        { path: "/settings/rules", element: <ShellRoute /> },
        { path: "/settings/plugins", element: <ShellRoute /> },
        { path: "/settings/notifications", element: <ShellRoute /> },
        { path: "/settings/subtitles", element: <ShellRoute /> },
        { path: "/system", element: <ShellRoute /> },
        { path: "*", element: <ShellRoute /> },
      ],
    },
  ],
  { basename: getRuntimeBasePath() },
);
