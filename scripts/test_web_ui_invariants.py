#!/usr/bin/env python3
from __future__ import annotations

import unittest
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
WEB_ROOT = ROOT / "apps" / "web"
SRC_ROOT = WEB_ROOT / "src"
APP_TSX = SRC_ROOT / "app" / "app.tsx"
ROUTER_TSX = SRC_ROOT / "app" / "router.tsx"
ROUTE_ELEMENTS_TSX = SRC_ROOT / "app" / "route-elements.tsx"
APP_SHELL_TSX = SRC_ROOT / "components" / "layout" / "app-shell.tsx"
APP_SHELL_NAV_TSX = SRC_ROOT / "components" / "layout" / "app-shell-nav.tsx"
I18N_CORE_TS = SRC_ROOT / "lib" / "i18n-core.ts"
I18N_CONTEXT_TS = SRC_ROOT / "lib" / "i18n-context.ts"
I18N_PROVIDER_TSX = SRC_ROOT / "lib" / "i18n-provider.tsx"
AGENTS_PAGE_TSX = SRC_ROOT / "features" / "agents" / "agents-page.tsx"
HOOKS_PAGE_TSX = SRC_ROOT / "features" / "hooks" / "hooks-page.tsx"
MANAGER_PAGE_TSX = SRC_ROOT / "features" / "manager" / "manager-page.tsx"
MANAGER_PREVIEW_HEADER_TOOLBAR_TSX = SRC_ROOT / "features" / "manager" / "manager-preview-header-toolbar.tsx"
COMPRESSION_PAGE_TSX = SRC_ROOT / "features" / "compression" / "compression-page.tsx"
COMPRESSION_ACTIONS_TSX = SRC_ROOT / "features" / "compression" / "compression-actions.tsx"
SYNC_PAGE_TSX = SRC_ROOT / "features" / "sync" / "sync-page.tsx"
SYNC_DETAIL_PAGE_TSX = SRC_ROOT / "features" / "sync" / "sync-detail-page.tsx"
WORKSPACE_SWITCH_DIALOG_TSX = SRC_ROOT / "features" / "workspaces" / "workspace-switch-dialog.tsx"
IMPORT_SESSION_DIALOG_TSX = SRC_ROOT / "features" / "import" / "import-session-dialog.tsx"
SETTINGS_DIALOG_TSX = SRC_ROOT / "features" / "settings" / "settings-dialog.tsx"
AGENT_ORDER_LIST_TSX = SRC_ROOT / "features" / "settings" / "agent-order-list.tsx"
HOME_PAGE_TSX = SRC_ROOT / "features" / "home" / "home-page.tsx"
SESSION_ACTION_TARGET_TS = SRC_ROOT / "features" / "sessions" / "session-action-target.ts"
SESSION_ACTIONS_TSX = SRC_ROOT / "features" / "sessions" / "session-actions.tsx"
SESSION_ACTIONS_DIR = SRC_ROOT / "features" / "sessions" / "actions"
SESSION_ACTION_SCHEMAS_TS = SRC_ROOT / "features" / "sessions" / "model" / "schemas.ts"
SESSION_DETAIL_PAGE_TSX = SRC_ROOT / "features" / "sessions" / "session-detail-page.tsx"
SESSION_DETAIL_HEADER_ACTIONS_TSX = SRC_ROOT / "features" / "sessions" / "session-detail-header-actions.tsx"
API_TS = SRC_ROOT / "lib" / "api.ts"
TYPES_TS = SRC_ROOT / "lib" / "types.ts"
UI_STORE_TS = SRC_ROOT / "stores" / "ui-store.ts"
HOME_QUERIES_TS = SRC_ROOT / "features" / "home" / "queries.ts"
QUERY_KEYS_TS = SRC_ROOT / "lib" / "query-keys.ts"
COMPONENTS_JSON = WEB_ROOT / "components.json"
def read_sources() -> str:
    parts = []
    for path in (APP_TSX, ROUTER_TSX, ROUTE_ELEMENTS_TSX, APP_SHELL_TSX, API_TS, UI_STORE_TS, HOME_QUERIES_TS, QUERY_KEYS_TS):
        if path.exists():
            parts.append(path.read_text(encoding="utf-8"))
    return "\n".join(parts)


def read_session_actions() -> str:
    paths = [
        SESSION_ACTIONS_DIR / "switch-session-dialog.tsx",
        SESSION_ACTIONS_DIR / "export-session-dialog.tsx",
        SESSION_ACTIONS_DIR / "create-sync-dialog.tsx",
        SESSION_ACTIONS_DIR / "rename-session-dialog.tsx",
        SESSION_ACTIONS_DIR / "delete-session-dialog.tsx",
        SESSION_ACTIONS_DIR / "index.ts",
        SESSION_ACTION_SCHEMAS_TS,
    ]
    return "\n".join(path.read_text(encoding="utf-8") for path in paths)


class WebUiInvariantTest(unittest.TestCase):
    def test_react_frontend_project_exists(self) -> None:
        self.assertTrue((WEB_ROOT / "package.json").exists())
        self.assertTrue(COMPONENTS_JSON.exists())
        self.assertTrue(APP_TSX.exists())
        self.assertTrue(APP_SHELL_TSX.exists())

    def test_app_uses_required_shared_providers(self) -> None:
        source = read_sources()
        self.assertIn("ThemeProvider", source)
        self.assertIn("QueryClientProvider", source)
        self.assertIn("I18nProvider", source)
        self.assertIn("TooltipProvider", source)
        self.assertIn("RouterProvider", source)
        self.assertIn("Toaster", source)

    def test_i18n_runtime_drives_shell_and_settings_language(self) -> None:
        self.assertTrue(I18N_CORE_TS.exists())
        self.assertTrue(I18N_CONTEXT_TS.exists())
        self.assertTrue(I18N_PROVIDER_TSX.exists())
        core = I18N_CORE_TS.read_text(encoding="utf-8")
        context = I18N_CONTEXT_TS.read_text(encoding="utf-8")
        provider = I18N_PROVIDER_TSX.read_text(encoding="utf-8")
        app = APP_TSX.read_text(encoding="utf-8")
        shell = APP_SHELL_TSX.read_text(encoding="utf-8") + APP_SHELL_NAV_TSX.read_text(encoding="utf-8")
        settings = SETTINGS_DIALOG_TSX.read_text(encoding="utf-8")

        for marker in [
            "export const dictionaries",
            'zh: {',
            'en: {',
            'settings: "设置"',
            'settings: "Settings"',
            'switchWorkspace: "切换路径"',
            'switchWorkspace: "Switch Workspace"',
            "navigator.language",
            "resolveLanguage",
        ]:
            self.assertIn(marker, core)

        for marker in [
            "queryKeys.meta",
            "settings.language",
            "setLanguageOverride",
            "document.documentElement.lang",
            '"zh-CN"',
        ]:
            self.assertIn(marker, provider)

        for marker in [
            "useI18n",
            "I18nContext",
        ]:
            self.assertIn(marker, context)

        self.assertIn("<I18nProvider>", app)
        self.assertIn("useI18n", shell)
        self.assertIn('t("switchWorkspace")', shell)
        self.assertIn('t("settings")', shell)
        self.assertIn("useI18n", settings)
        self.assertIn('t("language")', settings)
        self.assertIn('t("save")', settings)

    def test_router_covers_legacy_entry_points(self) -> None:
        source = read_sources()
        for route in [
            'path: "sessions"',
            'path: "sessions/:provider/:sessionId"',
            'path: "sync"',
            'path: "sync/:groupId"',
            'path: "manager"',
            'path: "compression"',
            'path: "agents"',
            'path: "tools"',
            'path: "hooks"',
        ]:
            self.assertIn(route, source)

    def test_router_uses_lazy_route_boundaries(self) -> None:
        source = read_sources()
        self.assertIn("lazy(() => import", source)
        self.assertIn("Suspense", source)
        self.assertIn("PageSkeleton", source)
        self.assertIn("LazyRoute", source)

    def test_shell_uses_legacy_topbar_contract(self) -> None:
        shell = APP_SHELL_TSX.read_text(encoding="utf-8") + APP_SHELL_NAV_TSX.read_text(encoding="utf-8")
        self.assertIn("Outlet", shell)
        self.assertIn("memorph", shell)
        for marker in [
            't("switchWorkspace")',
            't("hooks")',
            't("agentManagement")',
            't("manage")',
            't("compressSessions")',
            't("syncGroups")',
            't("importSession")',
            't("settings")',
        ]:
            self.assertIn(marker, shell)
        self.assertIn("w-[min(1280px,calc(100vw-24px))]", shell)
        self.assertIn("Button", shell)
        self.assertNotIn("Sheet", shell)
        for sidebar_component in ["SidebarProvider", "SidebarInset", "<Sidebar"]:
            self.assertNotIn(sidebar_component, shell)

    def test_workspace_switch_preserves_legacy_modal_workflow(self) -> None:
        shell = APP_SHELL_TSX.read_text(encoding="utf-8") + APP_SHELL_NAV_TSX.read_text(encoding="utf-8")
        dialog = WORKSPACE_SWITCH_DIALOG_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        home_queries = HOME_QUERIES_TS.read_text(encoding="utf-8")
        ui_store = UI_STORE_TS.read_text(encoding="utf-8")

        self.assertIn("WorkspaceSwitchDialog", shell)
        self.assertIn("setWorkspaceSwitchOpen(true)", shell)
        self.assertIn("data-workspace-switch-dialog", dialog)
        self.assertIn("data-workspace-switch-list", dialog)

        for marker in [
            "Switch Workspace",
            "Workspace Path",
            "known-workspaces",
            "Workspace History",
            "Browse",
            "Remove",
            "Go",
            "deleteWorkspaceHistory",
            "listSessions({ all: true, details: true, limit: 1, workspace })",
            "setSelectedWorkspace(workspace)",
        ]:
            self.assertIn(marker, dialog)

        self.assertIn('"/api/v1/workspaces/history"', api)
        self.assertIn('method: "DELETE"', api)
        self.assertIn("workspaces: [\"workspaces\"]", read_sources())
        self.assertIn("selectedWorkspace", ui_store)
        self.assertIn("workspace: selectedWorkspace", home_queries)
        self.assertIn("enabled: !meta.isLoading", home_queries)

    def test_import_session_preserves_legacy_modal_workflow(self) -> None:
        shell = APP_SHELL_TSX.read_text(encoding="utf-8") + APP_SHELL_NAV_TSX.read_text(encoding="utf-8")
        dialog = IMPORT_SESSION_DIALOG_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")
        query_keys = QUERY_KEYS_TS.read_text(encoding="utf-8")

        self.assertIn('import("@/features/import/import-session-dialog")', shell)
        self.assertIn("setImportSessionOpen(true)", shell)
        self.assertIn('t("importSession")', shell)
        self.assertIn("Suspense", shell)
        self.assertNotIn('from "@/features/import/import-session-dialog"', shell)

        for marker in [
            "data-import-session-dialog",
            "data-import-modal-grid",
            "Target Provider",
            "File Or Id",
            "Target Dir",
            "known-workspaces",
            "Browse",
            "Import",
            "selectFile",
            "selectFolder",
            "browseImportFile",
            "browseTargetDir",
            "zodResolver",
            "useForm",
            "useWatch",
            "importSession",
            "navigate(`/sessions/${encodeURIComponent(variables.provider)}/${encodeURIComponent(result.new_session_id)}`)",
        ]:
            self.assertIn(marker, dialog)

        for marker in [
            '"/api/v1/import"',
            'method: "POST"',
            "ImportSessionPayload",
            "ImportSessionResult",
        ]:
            self.assertIn(marker, api + types)

        self.assertIn("sessionsRoot", query_keys)

    def test_settings_preserves_legacy_wide_modal_workflow(self) -> None:
        shell = APP_SHELL_TSX.read_text(encoding="utf-8") + APP_SHELL_NAV_TSX.read_text(encoding="utf-8")
        dialog = SETTINGS_DIALOG_TSX.read_text(encoding="utf-8")
        order_list = AGENT_ORDER_LIST_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")
        query_keys = QUERY_KEYS_TS.read_text(encoding="utf-8")

        self.assertIn('import("@/features/settings/settings-dialog")', shell)
        self.assertIn("SettingsDialog", shell)
        self.assertIn("setSettingsOpen(true)", shell)
        self.assertIn("Suspense", shell)
        self.assertNotIn('from "@/features/settings/settings-dialog"', shell)
        self.assertNotIn("Sheet", shell)

        for marker in [
            "data-settings-dialog",
            "sm:max-w-3xl",
            "data-settings-layout",
            "data-settings-sidebar",
            'data-settings-section="general"',
            'data-settings-section="display"',
            'data-settings-section="order"',
            'data-settings-section="hook"',
            'data-settings-section="config"',
            'data-settings-section="about"',
            't("general")',
            't("display")',
            't("order")',
            't("hooks")',
            't("configFile")',
            't("configFileLocation")',
            't("about")',
            't("backupDir")',
            't("logDir")',
            't("logFileName")',
            't("logMaxSizeMb")',
            't("logRetentionDays")',
            't("sessionsPerProvider")',
            't("homeButtons")',
            't("hooks")',
            't("checkUpdate")',
            "PathText",
            "backup_dir_base",
            "updateSettings",
            "updateProviderCatalog",
            "getProviderCatalog",
            "checkForUpdate",
            "openExternal",
            "openUrl",
            "window.open",
        ]:
            self.assertIn(marker, dialog)

        for marker in [
            "AgentOrderList",
            't("hidden")',
            't("moveUp")',
            't("moveDown")',
            't("dragToReorder")',
            "SortableList",
        ]:
            self.assertIn(marker, dialog + order_list)

        for marker in [
            "UpdateSettingsPayload",
            "ProviderCatalogPayload",
            "ProviderCatalogUpdatePayload",
            "SelectPathPayload",
            "SelectPathResult",
            "OpenExternalPayload",
            "OpenExternalResult",
            "UpdateCheckPayload",
            '"/api/v1/settings"',
            '"/api/v1/providers/catalog"',
            '"/api/v1/system/select-folder"',
            '"/api/v1/system/select-file"',
            '"/api/v1/system/open-external"',
            '"/api/v1/update-check"',
            'method: "PUT"',
            'method: "POST"',
        ]:
            self.assertIn(marker, api + types)

        self.assertIn("providerCatalog", query_keys)

    def test_picker_update_and_external_link_api_boundaries_exist(self) -> None:
        api = API_TS.read_text(encoding="utf-8")
        import_dialog = IMPORT_SESSION_DIALOG_TSX.read_text(encoding="utf-8")
        settings_dialog = SETTINGS_DIALOG_TSX.read_text(encoding="utf-8")

        for marker in [
            "export function selectFolder",
            "export function selectFile",
            "export function openExternal",
            "export function checkForUpdate",
            '"/api/v1/system/select-folder"',
            '"/api/v1/system/select-file"',
            '"/api/v1/system/open-external"',
            '"/api/v1/update-check"',
        ]:
            self.assertIn(marker, api)

        self.assertIn("selectFile({ start_path", import_dialog)
        self.assertIn("selectFolder({ start_path", import_dialog)
        self.assertIn("File picker is only available in the desktop app.", import_dialog)
        self.assertIn("Folder picker is only available in the desktop app.", import_dialog)
        self.assertNotIn("disabled>Browse", import_dialog)

        self.assertIn('t("backupDir")', settings_dialog)
        self.assertIn("backup_dir_base", settings_dialog)
        self.assertIn("PathText", settings_dialog)
        self.assertNotIn("selectFolder({ start_path", settings_dialog)
        self.assertIn("mutationFn: checkForUpdate", settings_dialog)
        self.assertIn("await openExternal({ url })", settings_dialog)

    def test_session_rename_delete_preserve_legacy_row_and_detail_workflows(self) -> None:
        home = HOME_PAGE_TSX.read_text(encoding="utf-8")
        action_target = SESSION_ACTION_TARGET_TS.read_text(encoding="utf-8")
        actions = read_session_actions()
        detail = SESSION_DETAIL_PAGE_TSX.read_text(encoding="utf-8") + SESSION_DETAIL_HEADER_ACTIONS_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")

        for marker in [
            "onRename(session)",
            "onDelete(session)",
            "Rename",
            "Remove",
            "RenameSessionDialog",
            "DeleteSessionDialog",
            "targetFromSession(renameTarget)",
            "targetFromSession(deleteTarget)",
        ]:
            self.assertIn(marker, home)

        for marker in [
            "providerId: session.provider_id",
            "sessionId: session.session_id",
            "title: sessionTitle(session)",
            "workspace: session.project_dir",
        ]:
            self.assertIn(marker, action_target)

        for marker in [
            "data-rename-session-dialog",
            "data-delete-session-dialog",
            "DialogTitle>Rename",
            "AlertDialogTitle>Remove",
            "Title",
            "Save",
            "Remove",
            "renameSession(target.providerId, target.sessionId",
            "deleteSession(target.providerId, target.sessionId)",
            "queryKeys.sessionsRoot",
            "queryKeys.session(target.providerId, target.sessionId)",
            "queryKeys.home",
            "returnHomeOnSuccess",
            "navigate(\"/\")",
        ]:
            self.assertIn(marker, actions)

        for marker in [
            "setRenameOpen(true)",
            "setDeleteOpen(true)",
            "const actionTarget = { providerId: view.provider_id, sessionId: view.session_id, title: detailTitle(view), workspace: view.workspace_dir }",
            "target={actionTarget}",
            "returnHomeOnSuccess",
        ]:
            self.assertIn(marker, detail)

        for marker in [
            "RenameSessionPayload",
            "RenameSessionResult",
            'method: "PATCH"',
            'method: "DELETE"',
            "/api/v1/sessions/",
        ]:
            self.assertIn(marker, api + types)

    def test_session_copy_export_preserve_legacy_row_and_detail_workflows(self) -> None:
        home = HOME_PAGE_TSX.read_text(encoding="utf-8")
        actions = read_session_actions()
        detail = SESSION_DETAIL_PAGE_TSX.read_text(encoding="utf-8") + SESSION_DETAIL_HEADER_ACTIONS_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")

        for marker in [
            "onSwitch(session)",
            "onExport(session)",
            "Switch",
            "Export",
            "SwitchSessionDialog",
            "ExportSessionDialog",
            "targetFromSession(switchTarget)",
            "targetFromSession(exportTarget)",
            "providers={providers.data ?? []}",
            "meta={meta.data}",
        ]:
            self.assertIn(marker, home)

        for marker in [
            "data-switch-session-dialog",
            "data-switch-modal-grid",
            "Target Provider",
            "Copy Session Title",
            "Target Dir",
            "known-workspaces",
            "Browse",
            "Copy",
            "Move",
            "switchSession({",
            "move_original: moveOriginal",
            "navigate(`/sessions/${encodeURIComponent(variables.values.to)}/${encodeURIComponent(result.target_session_id)}`)",
            "data-export-session-dialog",
            "data-export-modal-grid",
            "Output File Name",
            "Format",
            "Export Directory",
            "exportSession({",
            "result.files.join",
        ]:
            self.assertIn(marker, actions)

        for marker in [
            "setSwitchOpen(true)",
            "setExportOpen(true)",
            "SwitchSessionDialog",
            "ExportSessionDialog",
            "providers={providers.data ?? []}",
            "meta={meta.data}",
            "Compression",
            "Sync",
            "Switch",
            "Export",
            "Rename",
            "Remove",
        ]:
            self.assertIn(marker, detail)

        for marker in [
            "ExportSessionPayload",
            "ExportSessionResult",
            "SwitchSessionPayload",
            "SwitchSessionResult",
            '"/api/v1/export"',
            '"/api/v1/switch"',
            'method: "POST"',
        ]:
            self.assertIn(marker, api + types)

    def test_create_sync_preserves_legacy_session_action_workflow(self) -> None:
        home = HOME_PAGE_TSX.read_text(encoding="utf-8")
        actions = read_session_actions()
        detail = SESSION_DETAIL_PAGE_TSX.read_text(encoding="utf-8") + SESSION_DETAIL_HEADER_ACTIONS_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")

        for marker in [
            "onSync(session)",
            "setSyncTarget",
            "CreateSyncDialog",
            "targetFromSession(syncTarget)",
            "providers={providers.data ?? []}",
            "meta={meta.data}",
        ]:
            self.assertIn(marker, home)

        for marker in [
            "data-create-sync-dialog",
            "data-create-sync-modal-stack",
            "data-create-sync-target-providers",
            "Create Sync",
            "Title",
            "Target Dir",
            "Target Providers",
            "known-workspaces",
            "Browse",
            "Create",
            "createSyncGroup({",
            "provider: target.providerId",
            "session_id: target.sessionId",
            "targets: values.targets.filter",
            "navigate(`/sync/${encodeURIComponent(group.id)}`)",
        ]:
            self.assertIn(marker, actions)

        for marker in [
            "setSyncOpen(true)",
            "CreateSyncDialog",
            "providers={providers.data ?? []}",
            "meta={meta.data}",
        ]:
            self.assertIn(marker, detail)

        for marker in [
            "CreateSyncPayload",
            '"/api/v1/sync"',
            'method: "POST"',
            "targets: string[]",
        ]:
            self.assertIn(marker, api + types)

    def test_agents_route_uses_real_provider_detail_page(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        agents_page = AGENTS_PAGE_TSX.read_text(encoding="utf-8")
        agents_route_match = re.search(
            r'\{\s*path: "agents",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIn('path: "agents"', router)
        self.assertIsNotNone(agents_route_match)
        agents_route_element = agents_route_match.group("element") if agents_route_match else ""
        self.assertIn("<AgentsPage />", agents_route_element)
        self.assertNotIn("MigrationPage", agents_route_element)
        self.assertIn('import("@/features/agents/agents-page")', route_elements)

        for marker in [
            "ProviderList",
            "ProviderDetail",
            "Agent Management Environment",
            "Open Hooks",
            "Detect",
            "Agent Provider Items",
            "useDetectAgent",
            "useUpdateProviderSetting",
            "useRunProviderSetting",
        ]:
            self.assertIn(marker, agents_page)

    def test_hooks_route_uses_real_provider_diagnostics_page(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        hooks_page = HOOKS_PAGE_TSX.read_text(encoding="utf-8")
        hooks_route_match = re.search(
            r'\{\s*path: "hooks",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIsNotNone(hooks_route_match)
        hooks_route_element = hooks_route_match.group("element") if hooks_route_match else ""
        self.assertIn("<HooksPage />", hooks_route_element)
        self.assertNotIn("MigrationPage", hooks_route_element)
        self.assertIn('import("@/features/hooks/hooks-page")', route_elements)

        for marker in [
            "HooksProviderList",
            "ProviderDetail",
            "Hook Summary",
            "Hook Event Profile",
            "Runtime Sessions",
            "Recent Events",
            "Recent Errors",
            "useHooksOverview",
            "useHookProviderOverview",
            "useRunHookProviderOperation",
        ]:
            self.assertIn(marker, hooks_page)

    def test_manager_route_preserves_legacy_two_panel_preview(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        manager_page = MANAGER_PAGE_TSX.read_text(encoding="utf-8") + MANAGER_PREVIEW_HEADER_TOOLBAR_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")
        manager_route_match = re.search(
            r'\{\s*path: "manager",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIsNotNone(manager_route_match)
        manager_route_element = manager_route_match.group("element") if manager_route_match else ""
        self.assertIn("<ManagerPage />", manager_route_element)
        self.assertNotIn("MigrationPage", manager_route_element)
        self.assertIn('import("@/features/manager/manager-page")', route_elements)

        for marker in [
            "data-manager-page-layout",
            "data-manager-control-panel",
            "data-manager-result-panel",
            "data-manager-workspace-summary",
            "data-manager-provider-controls",
            "data-manager-view-tabs",
            "data-manager-preview-header",
            "data-manager-preview-search",
            "data-manager-action-clean",
            "data-manager-action-backup",
            "data-manager-action-more",
            "data-manager-selection-menu",
            "data-manager-row",
            "data-manager-row-actions",
            "SessionRows",
            "WorkspaceRows",
            "Clean Selected",
            "Select All",
            "data-manager-action-dialog",
            "data-manager-clean-dialog",
            "data-manager-backup-dialog",
            "data-manager-action-result",
            "cleanManagerItems",
            "backupManagerItems",
            "cleanManagerWorkspace",
            "backupManagerWorkspace",
            "queryClient.invalidateQueries({ queryKey: [\"manager\"] })",
        ]:
            self.assertIn(marker, manager_page)

        for marker in [
            "ManagerItemsPayload",
            "ManagerWorkspacePayload",
            "ManagerCleanResult",
            "ManagerBackupResult",
            '"/api/v1/manager/clean"',
            '"/api/v1/manager/backup"',
            '"/api/v1/manager/clean-workspace"',
            '"/api/v1/manager/backup-workspace"',
            'method: "POST"',
        ]:
            self.assertIn(marker, api + types)

        self.assertNotIn("<Table", manager_page)

    def test_compression_route_preserves_legacy_two_panel_row_workflow(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        compression_page = COMPRESSION_PAGE_TSX.read_text(encoding="utf-8")
        compression_actions = COMPRESSION_ACTIONS_TSX.read_text(encoding="utf-8")
        home = HOME_PAGE_TSX.read_text(encoding="utf-8")
        detail = SESSION_DETAIL_PAGE_TSX.read_text(encoding="utf-8") + SESSION_DETAIL_HEADER_ACTIONS_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")
        compression_route_match = re.search(
            r'\{\s*path: "compression",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIsNotNone(compression_route_match)
        compression_route_element = compression_route_match.group("element") if compression_route_match else ""
        self.assertIn("<CompressionPage />", compression_route_element)
        self.assertNotIn("MigrationPage", compression_route_element)
        self.assertIn('import("@/features/compression/compression-page")', route_elements)

        for marker in [
            "data-manager-page-layout",
            "data-manager-control-panel",
            "data-manager-result-panel",
            "data-compression-candidate-row",
            "data-compression-archive-row",
            "data-compression-workspace-summary",
            "data-compression-provider-support",
            "Compress Sessions",
            "Compression Archives",
            "View",
            "Compression",
            "Restore",
            "data-restore-compression-dialog",
            "data-compression-restore-archive-ref",
            'name="archive_ref"',
            "Output Prefix",
            "defaultRestorePrefix",
            "useRestoreCompressionArchive",
        ]:
            self.assertIn(marker, compression_page)

        for marker in [
            "data-session-header",
            "data-detail-layout",
            "data-compression-detail-layout",
            "data-detail-timeline",
            "Compression Archive Detail",
        ]:
            self.assertIn(marker, compression_page)

        for forbidden in ["<Table", "<Tabs", "function StatCard", "<StatCard"]:
            self.assertNotIn(forbidden, compression_page)

        for marker in [
            "data-compress-session-dialog",
            "data-compression-path-line",
            "compressionArchiveHref",
            "primaryArchiveRef",
            'to={compressionArchiveHref(primaryArchiveRef)}',
            "Open Archive",
            "useApplyCompression",
        ]:
            self.assertIn(marker, compression_actions)

        for marker in [
            "onCompress(session)",
            "setCompressionTarget",
            "CompressSessionDialog",
            "targetFromSession(compressionTarget)",
        ]:
            self.assertIn(marker, home)

        for marker in [
            "data-session-detail-scroll",
            "<ScrollArea className=\"h-full pr-3\"",
            "setCompressionOpen(true)",
            "CompressSessionDialog",
            "target={actionTarget}",
        ]:
            self.assertIn(marker, detail)

        for marker in [
            "ApplyCompressionPayload",
            "ApplyCompressionResult",
            "RestoreCompressionPayload",
            "RestoreCompressionResult",
            '"/api/v1/compression/apply"',
            '"/api/v1/compression/restore"',
            'method: "POST"',
        ]:
            self.assertIn(marker, api + types)

    def test_sync_route_preserves_legacy_row_workflow(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        sync_page = SYNC_PAGE_TSX.read_text(encoding="utf-8")
        api = API_TS.read_text(encoding="utf-8")
        types = TYPES_TS.read_text(encoding="utf-8")
        sync_route_match = re.search(
            r'\{\s*path: "sync",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIsNotNone(sync_route_match)
        sync_route_element = sync_route_match.group("element") if sync_route_match else ""
        self.assertIn("<SyncPage />", sync_route_element)
        self.assertNotIn("MigrationPage", sync_route_element)
        self.assertIn('import("@/features/sync/sync-page")', route_elements)

        for marker in [
            "data-sync-list-layout",
            "data-sync-control-panel",
            "data-sync-result-panel",
            "data-sync-row-list",
            "data-sync-row-actions",
            "View",
            "Sync Latest",
            "Rename",
            "Remove",
            "RenameSyncGroupDialog",
            "RemoveSyncGroupDialog",
            "data-rename-sync-group-dialog",
            "data-remove-sync-group-dialog",
            "delete_provider_sessions",
            "runSyncGroup({ group_id: group.id })",
        ]:
            self.assertIn(marker, sync_page)

        self.assertNotIn("<Table", sync_page)

        for marker in [
            "RenameSyncGroupPayload",
            "SyncRunPayload",
            "SyncReport",
            "BindSyncPayload",
            "renameSyncGroup",
            "removeSyncGroup",
            "bindSyncGroup",
            "runSyncGroup",
            "unbindSyncHolding",
            '"/api/v1/sync/sync"',
            '"/api/v1/sync/bind"',
            '/api/v1/sync/holdings/',
            'delete_provider_sessions',
            'method: "PATCH"',
            'method: "DELETE"',
            'method: "POST"',
        ]:
            self.assertIn(marker, api + types)

    def test_sync_detail_route_preserves_legacy_holding_workflow(self) -> None:
        router = ROUTER_TSX.read_text(encoding="utf-8")
        route_elements = ROUTE_ELEMENTS_TSX.read_text(encoding="utf-8")
        sync_detail = SYNC_DETAIL_PAGE_TSX.read_text(encoding="utf-8")
        sync_detail_route_match = re.search(
            r'\{\s*path: "sync/:groupId",\s*element: (?P<element>.*?)\s*\}',
            router,
            re.DOTALL,
        )

        self.assertIsNotNone(sync_detail_route_match)
        sync_detail_route_element = sync_detail_route_match.group("element") if sync_detail_route_match else ""
        self.assertIn("<SyncDetailPage />", sync_detail_route_element)
        self.assertNotIn("MigrationPage", sync_detail_route_element)
        self.assertIn('import("@/features/sync/sync-detail-page")', route_elements)

        for marker in [
            "data-sync-detail-layout",
            "data-sync-holdings-panel",
            "data-sync-detail-actions",
            "data-sync-holding-grid",
            "data-sync-holding-card",
            "data-sync-holding-actions",
            "Open Session",
            "Sync From This",
            "Unbind",
            "Start Execution",
            "Add Holding",
            "Rename",
            "Remove",
            "data-bind-sync-holding-dialog",
            "data-bind-sync-modal-stack",
            "data-sync-from-holding-dialog",
            "data-unbind-sync-holding-dialog",
            "data-rename-sync-group-dialog",
            "data-remove-sync-group-dialog",
            "delete_provider_sessions",
            "bindSyncGroup({",
            "runSyncGroup({ group_id: group.id, source_holding_id: holding.id })",
            "unbindSyncHolding(group.id, holding.id)",
            "removeSyncGroup(group.id, deleteProviderSessions)",
            "navigate(\"/sync\")",
        ]:
            self.assertIn(marker, sync_detail)

        self.assertNotIn("<Table", sync_detail)

    def test_api_client_and_state_boundary_exist(self) -> None:
        source = read_sources()
        self.assertIn("export async function api", source)
        self.assertIn("ApiError", source)
        self.assertIn("create", source)
        self.assertIn("zustand", source)

    def test_old_legacy_web_source_is_not_active(self) -> None:
        self.assertFalse((ROOT / "web").exists())
        self.assertFalse((ROOT / "web-legacy").exists())
        source = read_sources()
        self.assertNotIn("web-legacy", source)


if __name__ == "__main__":
    unittest.main(verbosity=2)
