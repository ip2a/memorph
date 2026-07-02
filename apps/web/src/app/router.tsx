import { createBrowserRouter } from "react-router-dom";
import { AppShell } from "@/components/layout/app-shell";
import { migrationPages } from "@/features/migration/migration-pages";
import { AgentsPage, CompressionPage, HomePage, HooksPage, LazyRoute, ManagerPage, MigrationPage, SessionDetailPage, SessionsPage, SyncDetailPage, SyncPage } from "@/app/route-elements";

export const router = createBrowserRouter([
  {
    path: "/",
    element: <AppShell />,
    children: [
      {
        index: true,
        element: <LazyRoute><HomePage /></LazyRoute>,
      },
      {
        path: "sessions",
        element: <LazyRoute><SessionsPage /></LazyRoute>,
      },
      {
        path: "sessions/:provider/:sessionId",
        element: <LazyRoute><SessionDetailPage /></LazyRoute>,
      },
      {
        path: "sync",
        element: <LazyRoute><SyncPage /></LazyRoute>,
      },
      {
        path: "sync/:groupId",
        element: <LazyRoute><SyncDetailPage /></LazyRoute>,
      },
      {
        path: "manager",
        element: <LazyRoute><ManagerPage /></LazyRoute>,
      },
      {
        path: "compression",
        element: <LazyRoute><CompressionPage /></LazyRoute>,
      },
      {
        path: "agents",
        element: <LazyRoute><AgentsPage /></LazyRoute>,
      },
      {
        path: "tools",
        element: <LazyRoute><MigrationPage {...migrationPages.agents} /></LazyRoute>,
      },
      {
        path: "hooks",
        element: <LazyRoute><HooksPage /></LazyRoute>,
      },
      {
        path: "*",
        element: <LazyRoute><HomePage /></LazyRoute>,
      },
    ],
  },
]);
