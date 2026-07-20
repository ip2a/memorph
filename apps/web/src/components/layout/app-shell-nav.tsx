import * as React from "react"
import { Link, useLocation, useNavigate } from "react-router-dom"
import { ArrowLeftIcon, DatabaseIcon, PackageIcon, SettingsIcon } from "lucide-react"

import { CollapsibleToolbar, type CollapsibleToolbarEntry } from "@/components/shared/collapsible-toolbar"
import { Button } from "@/components/ui/button"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { useI18n } from "@/lib/i18n-context"
import { useUiStore } from "@/stores/ui-store"

function isRoute(pathname: string, route: string) {
  return route === "/" ? pathname === "/" : pathname.startsWith(route)
}

export function AppShellNav({
  onOpenImportSession,
  onOpenSettings,
}: {
  onOpenImportSession: () => void
  onOpenSettings: () => void
}) {
  const location = useLocation()
  const navigate = useNavigate()
  const { t } = useI18n()
  const setWorkspaceSwitchOpen = useUiStore((state) => state.setWorkspaceSwitchOpen)

  const pathname = location.pathname
  const isHome = pathname === "/"
  const isManager = isRoute(pathname, "/manager")
  const isHooks = isRoute(pathname, "/hooks")
  const isSkills = isRoute(pathname, "/skills")
  const isAgents = isRoute(pathname, "/agents")
  const isStats = isRoute(pathname, "/stats")
  const isStorage = isRoute(pathname, "/storage")

  const entries = React.useMemo<CollapsibleToolbarEntry[]>(() => {
    const next: CollapsibleToolbarEntry[] = []

    if (!isHome) {
      next.push({
        id: "back",
        collapsePriority: 0,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={() => navigate(-1)}>
            <ArrowLeftIcon data-icon="inline-start" />
            {t("back")}
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem onSelect={() => navigate(-1)}>
            <ArrowLeftIcon />
            {t("back")}
          </DropdownMenuItem>
        ),
      })
    }

    next.push({
      id: "switch-workspace",
      collapsePriority: 8,
      renderButton: () => (
        <Button type="button" variant="outline" onClick={() => setWorkspaceSwitchOpen(true)}>
          {t("switchWorkspace")}
        </Button>
      ),
      renderMenuItem: () => (
        <DropdownMenuItem onSelect={() => setWorkspaceSwitchOpen(true)}>{t("switchWorkspace")}</DropdownMenuItem>
      ),
    })

    if (!isStats) {
      next.push({
        id: "stats",
        collapsePriority: 10,
        renderButton: () => (
          <Button asChild variant="outline">
            <Link to="/stats">{t("stats")}</Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/stats">{t("stats")}</Link>
          </DropdownMenuItem>
        ),
      })
    }

    if (!isStorage) {
      next.push({
        id: "storage",
        collapsePriority: 10,
        renderButton: () => (
          <Button asChild variant="outline">
            <Link to="/storage">
              <DatabaseIcon data-icon="inline-start" />
              {t("storage")}
            </Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/storage">
              <DatabaseIcon />
              {t("storage")}
            </Link>
          </DropdownMenuItem>
        ),
      })
    }

    if (!isHooks) {
      next.push({
        id: "hooks",
        collapsePriority: 11,
        renderButton: () => (
          <Button asChild variant="outline">
            <Link to="/hooks">{t("hooks")}</Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/hooks">{t("hooks")}</Link>
          </DropdownMenuItem>
        ),
      })
    }

    if (!isSkills) {
      next.push({
        id: "skills",
        collapsePriority: 12,
        renderButton: () => (
          <Button asChild variant="outline" size="sm">
            <Link to="/skills">
              <PackageIcon data-icon="inline-start" />
              {t("skills")}
            </Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/skills">
              <PackageIcon />
              {t("skills")}
            </Link>
          </DropdownMenuItem>
        ),
      })
    }

    if (!isAgents) {
      next.push({
        id: "agents",
        collapsePriority: 12,
        renderButton: () => (
          <Button asChild variant="outline">
            <Link to="/agents">{t("agentManagement")}</Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/agents">{t("agentManagement")}</Link>
          </DropdownMenuItem>
        ),
      })
    }

    if (isManager) {
      next.push(
        {
          id: "compression",
          collapsePriority: 20,
          renderButton: () => (
            <Button asChild variant="outline">
              <Link to="/compression">{t("compressSessions")}</Link>
            </Button>
          ),
          renderMenuItem: () => (
            <DropdownMenuItem asChild>
              <Link to="/compression">{t("compressSessions")}</Link>
            </DropdownMenuItem>
          ),
        },
        {
          id: "sync",
          collapsePriority: 21,
          renderButton: () => (
            <Button asChild variant="outline">
              <Link to="/sync">{t("syncGroups")}</Link>
            </Button>
          ),
          renderMenuItem: () => (
            <DropdownMenuItem asChild>
              <Link to="/sync">{t("syncGroups")}</Link>
            </DropdownMenuItem>
          ),
        },
        {
          id: "import-session",
          collapsePriority: 22,
          renderButton: () => (
            <Button type="button" variant="outline" onClick={onOpenImportSession}>
              {t("importSession")}
            </Button>
          ),
          renderMenuItem: () => (
            <DropdownMenuItem onSelect={onOpenImportSession}>{t("importSession")}</DropdownMenuItem>
          ),
        },
      )
    } else {
      next.push({
        id: "manage",
        collapsePriority: 15,
        renderButton: () => (
          <Button asChild variant="outline">
            <Link to="/manager">{t("manage")}</Link>
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem asChild>
            <Link to="/manager">{t("manage")}</Link>
          </DropdownMenuItem>
        ),
      })
    }

    next.push({
      id: "settings",
      collapsePriority: 60,
      renderButton: () => (
        <Button variant="outline" onClick={onOpenSettings}>
          <SettingsIcon data-icon="inline-start" />
          {t("settings")}
        </Button>
      ),
      renderMenuItem: () => (
        <DropdownMenuItem onSelect={onOpenSettings}>
          <SettingsIcon />
          {t("settings")}
        </DropdownMenuItem>
      ),
    })

    return next
  }, [
    isAgents,
    isHome,
    isHooks,
    isSkills,
    isManager,
    isStats,
    isStorage,
    navigate,
    onOpenImportSession,
    onOpenSettings,
    setWorkspaceSwitchOpen,
    t,
  ])

  return <CollapsibleToolbar className="min-w-0 flex-1" entries={entries} moreLabel={t("more")} />
}
