import { lazy, Suspense } from "react";
import { createBrowserRouter } from "react-router-dom";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { RouteErrorBoundary } from "./error-boundary";
import type { HomePageRouteState } from "@/lib/types/root";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import type { ViewId } from "@/components/root/types";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));

function ShellRoute({
  initialView,
  initialSettingsSection,
  initialContentSection,
}: HomePageRouteState) {
  return (
    <Suspense fallback={<PageShellFallback />}>
      <RootPageShell
        initialView={initialView}
        initialSettingsSection={initialSettingsSection}
        initialContentSection={initialContentSection}
      />
    </Suspense>
  );
}

const mediaRoutes = FACET_REGISTRY.flatMap((f) => {
  const viewId = f.viewId as ViewId;
  return [
    { path: `/${f.viewId}`, element: <ShellRoute initialView={viewId} initialContentSection="overview" /> },
    { path: `/${f.viewId}/overview`, element: <ShellRoute initialView={viewId} initialContentSection="overview" /> },
    { path: `/${f.viewId}/settings`, element: <ShellRoute initialView={viewId} initialContentSection="settings" /> },
  ];
});

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
      ...mediaRoutes,
      { path: "/activity", element: <ShellRoute initialView="activity" /> },
      { path: "/wanted", element: <ShellRoute initialView="wanted" /> },
      { path: "/settings", element: <ShellRoute initialView="settings" initialSettingsSection="profile" /> },
      { path: "/settings/profile", element: <ShellRoute initialView="settings" initialSettingsSection="profile" /> },
      { path: "/settings/indexers", element: <ShellRoute initialView="settings" initialSettingsSection="indexers" /> },
      {
        path: "/settings/download-clients",
        element: <ShellRoute initialView="settings" initialSettingsSection="downloadClients" />,
      },
      {
        path: "/settings/quality-profiles",
        element: <ShellRoute initialView="settings" initialSettingsSection="qualityProfiles" />,
      },
      { path: "/settings/users", element: <ShellRoute initialView="settings" initialSettingsSection="users" /> },
      {
        path: "/settings/acquisition",
        element: <ShellRoute initialView="settings" initialSettingsSection="acquisition" />,
      },
      { path: "/system", element: <ShellRoute initialView="system" /> },
      { path: "*", element: <ShellRoute /> },
    ],
  },
]);
