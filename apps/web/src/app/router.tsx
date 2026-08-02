import { createBrowserRouter } from "react-router-dom";
import { AppShell } from "@/components/layout/app-shell";
import { migrationPages } from "@/features/migration/migration-pages";
import { AgentsPage, ArtifactsPage, CompressionPage, HomePage, LazyRoute, SkillsPage, ManagerPage, MigrationPage, SessionDetailPage, SessionsPage, StatsPage, SyncDetailPage, SyncPage } from "@/app/route-elements";

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
        path: "storage",
        element: <LazyRoute><ArtifactsPage /></LazyRoute>,
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
        path: "skills",
        element: <LazyRoute><SkillsPage /></LazyRoute>,
      },
      {
        path: "stats",
        element: <LazyRoute><StatsPage /></LazyRoute>,
      },
      {
        path: "*",
        element: <LazyRoute><HomePage /></LazyRoute>,
      },
    ],
  },
]);
