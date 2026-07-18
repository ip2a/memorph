import { lazy, Suspense, type ReactNode } from "react";
import { PageSkeleton } from "@/components/shared/page-states";

export const HomePage = lazy(() => import("@/features/home/home-page").then((module) => ({ default: module.HomePage })));
export const SessionsPage = lazy(() =>
  import("@/features/sessions/sessions-page").then((module) => ({ default: module.SessionsPage })),
);
export const SessionDetailPage = lazy(() =>
  import("@/features/sessions/session-detail-page").then((module) => ({ default: module.SessionDetailPage })),
);
export const SyncPage = lazy(() => import("@/features/sync/sync-page").then((module) => ({ default: module.SyncPage })));
export const SyncDetailPage = lazy(() =>
  import("@/features/sync/sync-detail-page").then((module) => ({ default: module.SyncDetailPage })),
);
export const ManagerPage = lazy(() =>
  import("@/features/manager/manager-page").then((module) => ({ default: module.ManagerPage })),
);
export const CompressionPage = lazy(() =>
  import("@/features/compression/compression-page").then((module) => ({ default: module.CompressionPage })),
);
export const ArtifactsPage = lazy(() =>
  import("@/features/artifacts/artifacts-page").then((module) => ({ default: module.ArtifactsPage })),
);
export const AgentsPage = lazy(() =>
  import("@/features/agents/agents-page").then((module) => ({ default: module.AgentsPage })),
);
export const HooksPage = lazy(() => import("@/features/hooks/hooks-page").then((module) => ({ default: module.HooksPage })));
export const SkillsPage = lazy(() => import("@/features/skills/skills-page").then((module) => ({ default: module.SkillsPage })));
export const StatsPage = lazy(() => import("@/features/stats/stats-page").then((module) => ({ default: module.StatsPage })));
export const MigrationPage = lazy(() =>
  import("@/features/migration/migration-page").then((module) => ({ default: module.MigrationPage })),
);

export function LazyRoute({ children }: { children: ReactNode }) {
  return <Suspense fallback={<PageSkeleton />}>{children}</Suspense>;
}
