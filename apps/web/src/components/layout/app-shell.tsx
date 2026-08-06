import { Link, Outlet, useLocation } from "react-router-dom";
import { lazy, Suspense, useState } from "react";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { cn } from "@/lib/utils";
import { AppShellNav } from "@/components/layout/app-shell-nav";
import { WorkspaceSwitchDialog } from "@/features/workspaces/workspace-switch-dialog";
import { WorkspaceQuickSwitchDialog } from "@/features/workspaces/workspace-quick-switch-dialog";
import { useUiStore } from "@/stores/ui-store";
import { ReadinessIndicator } from "@/features/readiness/readiness-indicator";

const ImportSessionDialog = lazy(() =>
  import("@/features/import/import-session-dialog").then((module) => ({ default: module.ImportSessionDialog })),
);

const SettingsDialog = lazy(() =>
  import("@/features/settings/settings-dialog").then((module) => ({ default: module.SettingsDialog })),
);

function isFullscreenRoute(pathname: string) {
  if (pathname === "/") return true;
  if (pathname.startsWith("/sessions/")) return true;
  if (pathname.startsWith("/sessions")) return false;
  if (pathname.startsWith("/sync")) return true;
  if (pathname.startsWith("/manager")) return true;
  if (pathname.startsWith("/compression")) return true;
  if (pathname.startsWith("/storage")) return true;
  if (pathname.startsWith("/agents")) return true;
  if (pathname.startsWith("/stats")) return true;
  if (pathname.startsWith("/tools")) return false;
  return true;
}

function routeTitleKey(pathname: string): I18nKey {
  if (pathname.startsWith("/sessions/")) return "sessionDetail";
  if (pathname.startsWith("/sync/")) return "syncDetail";
  if (pathname.startsWith("/sessions")) return "sessions";
  if (pathname.startsWith("/sync")) return "sync";
  if (pathname.startsWith("/manager")) return "manage";
  if (pathname.startsWith("/compression")) return "compression";
  if (pathname.startsWith("/storage")) return "storage";
  if (pathname.startsWith("/agents")) return "agentManagement";
  if (pathname.startsWith("/stats")) return "stats";
  return "home";
}

export function AppShell() {
  const location = useLocation();
  const { t } = useI18n();
  const workspaceSwitchOpen = useUiStore((state) => state.workspaceSwitchOpen);
  const setWorkspaceSwitchOpen = useUiStore((state) => state.setWorkspaceSwitchOpen);
  const [importSessionOpen, setImportSessionOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const fullscreen = isFullscreenRoute(location.pathname);
  const title = t(routeTitleKey(location.pathname));

  return (
    <div className="mx-auto grid h-dvh w-[min(1280px,calc(100vw-24px))] grid-cols-[minmax(0,1fr)] grid-rows-[auto_minmax(0,1fr)] overflow-hidden">
      <header className="flex min-h-16 min-w-0 items-center justify-between gap-4 border-b py-3">
        <div className="flex min-w-0 shrink items-center gap-3">
          <Link to="/" className="font-mono font-bold">
            memorph
          </Link>
          {title ? (
            <>
              <div className="h-5 w-px bg-border" />
              <span className="truncate text-sm font-semibold">{title}</span>
            </>
          ) : null}
        </div>

        <div className="flex min-w-0 flex-1 items-center justify-end gap-3">
          <ReadinessIndicator />
          <AppShellNav
            onOpenImportSession={() => setImportSessionOpen(true)}
            onOpenSettings={() => setSettingsOpen(true)}
          />
        </div>
      </header>

      <WorkspaceSwitchDialog open={workspaceSwitchOpen} onOpenChange={setWorkspaceSwitchOpen} />
      <WorkspaceQuickSwitchDialog />
      {importSessionOpen ? (
        <Suspense fallback={null}>
          <ImportSessionDialog open={importSessionOpen} onOpenChange={setImportSessionOpen} />
        </Suspense>
      ) : null}
      {settingsOpen ? (
        <Suspense fallback={null}>
          <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
        </Suspense>
      ) : null}

      <main
        className={cn(
          "min-h-0 min-w-0",
          fullscreen ? "flex h-full flex-col overflow-hidden pt-3" : "overflow-auto py-3",
        )}
      >
        <Outlet />
      </main>
    </div>
  );
}
