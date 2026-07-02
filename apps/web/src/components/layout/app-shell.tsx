import { Link, Outlet, useLocation, useNavigate } from "react-router-dom";
import { lazy, Suspense, useState } from "react";
import { MoreHorizontalIcon, SettingsIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { cn } from "@/lib/utils";
import { WorkspaceSwitchDialog } from "@/features/workspaces/workspace-switch-dialog";

const ImportSessionDialog = lazy(() =>
  import("@/features/import/import-session-dialog").then((module) => ({ default: module.ImportSessionDialog })),
);

const SettingsDialog = lazy(() =>
  import("@/features/settings/settings-dialog").then((module) => ({ default: module.SettingsDialog })),
);

function isRoute(pathname: string, route: string) {
  return route === "/" ? pathname === "/" : pathname.startsWith(route);
}

function routeTitleKey(pathname: string): I18nKey {
  if (pathname.startsWith("/sessions/")) return "sessionDetail";
  if (pathname.startsWith("/sync/")) return "syncDetail";
  if (pathname.startsWith("/sessions")) return "sessions";
  if (pathname.startsWith("/sync")) return "sync";
  if (pathname.startsWith("/manager")) return "manage";
  if (pathname.startsWith("/compression")) return "compression";
  if (pathname.startsWith("/agents")) return "agentManagement";
  if (pathname.startsWith("/hooks")) return "hooks";
  return "home";
}

export function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { t } = useI18n();
  const [workspaceSwitchOpen, setWorkspaceSwitchOpen] = useState(false);
  const [importSessionOpen, setImportSessionOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const isHome = location.pathname === "/";
  const isManager = isRoute(location.pathname, "/manager");
  const isHooks = isRoute(location.pathname, "/hooks");
  const isAgents = isRoute(location.pathname, "/agents");
  const title = t(routeTitleKey(location.pathname));

  return (
    <div className="mx-auto grid h-dvh w-[min(1280px,calc(100vw-24px))] grid-rows-[auto_minmax(0,1fr)] overflow-hidden pb-4">
      <header className="mb-3 flex min-h-16 items-center justify-between gap-4 border-b py-3">
        <div className="flex min-w-0 items-center gap-3">
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

        <nav className="flex flex-wrap items-center justify-end gap-2">
          {!isHome ? (
            <Button type="button" variant="outline" size="sm" onClick={() => navigate(-1)}>
              {t("back")}
            </Button>
          ) : null}
          <Button type="button" variant="outline" size="sm" onClick={() => setWorkspaceSwitchOpen(true)}>
            {t("switchWorkspace")}
          </Button>
          {!isHooks ? (
            <Button asChild variant="outline" size="sm">
              <Link to="/hooks">{t("hooks")}</Link>
            </Button>
          ) : null}
          {!isAgents ? (
            <Button asChild variant="outline" size="sm">
              <Link to="/agents">{t("agentManagement")}</Link>
            </Button>
          ) : null}
          {isManager ? (
            <>
              <Button asChild variant="outline" size="sm">
                <Link to="/compression">{t("compressSessions")}</Link>
              </Button>
              <Button asChild variant="outline" size="sm">
                <Link to="/sync">{t("syncGroups")}</Link>
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => setImportSessionOpen(true)}>
                {t("importSession")}
              </Button>
            </>
          ) : (
            <Button asChild variant="outline" size="sm">
              <Link to="/manager">{t("manage")}</Link>
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={() => setSettingsOpen(true)}>
            <SettingsIcon data-icon="inline-start" />
            {t("settings")}
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label={t("openMoreActions")}>
                <MoreHorizontalIcon />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuLabel>{t("actions")}</DropdownMenuLabel>
              <DropdownMenuGroup>
                <DropdownMenuItem onSelect={() => setImportSessionOpen(true)}>{t("importSession")}</DropdownMenuItem>
                <DropdownMenuItem>{t("exportSession")}</DropdownMenuItem>
                <DropdownMenuItem>{t("checkUpdate")}</DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem>{t("openRepository")}</DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </nav>
      </header>

      <WorkspaceSwitchDialog open={workspaceSwitchOpen} onOpenChange={setWorkspaceSwitchOpen} />
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
          "min-h-0 overflow-hidden",
          isRoute(location.pathname, "/manager") || isRoute(location.pathname, "/agents") ? "pb-0" : "",
        )}
      >
        <Outlet />
      </main>
    </div>
  );
}
