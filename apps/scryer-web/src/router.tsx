import { lazy, Suspense } from "react";
import { createBrowserRouter } from "react-router-dom";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { RouteErrorBoundary } from "./error-boundary";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));

function ShellRoute() {
  return (
    <Suspense fallback={<PageShellFallback />}>
      <RootPageShell />
    </Suspense>
  );
}

export const router = createBrowserRouter([
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
      { path: "/", element: <ShellRoute /> },
      { path: "/movies", element: <ShellRoute /> },
      { path: "/movies/overview", element: <ShellRoute /> },
      { path: "/movies/settings", element: <ShellRoute /> },
      { path: "/series", element: <ShellRoute /> },
      { path: "/series/overview", element: <ShellRoute /> },
      { path: "/series/settings", element: <ShellRoute /> },
      { path: "/anime", element: <ShellRoute /> },
      { path: "/anime/overview", element: <ShellRoute /> },
      { path: "/anime/settings", element: <ShellRoute /> },
      { path: "/activity", element: <ShellRoute /> },
      { path: "/wanted", element: <ShellRoute /> },
      { path: "/settings", element: <ShellRoute /> },
      { path: "/settings/profile", element: <ShellRoute /> },
      { path: "/settings/indexers", element: <ShellRoute /> },
      { path: "/settings/download-clients", element: <ShellRoute /> },
      { path: "/settings/quality-profiles", element: <ShellRoute /> },
      { path: "/settings/users", element: <ShellRoute /> },
      { path: "/settings/acquisition", element: <ShellRoute /> },
      { path: "/settings/post-processing", element: <ShellRoute /> },
      { path: "/settings/rules", element: <ShellRoute /> },
      { path: "/system", element: <ShellRoute /> },
      { path: "*", element: <ShellRoute /> },
    ],
  },
]);
