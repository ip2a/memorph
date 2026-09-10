// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ManagerPage } from "./manager-page";
import { I18nContext } from "@/lib/i18n-context";
import { translate } from "@/lib/i18n-core";
import { useUiStore } from "@/stores/ui-store";

const mocks = vi.hoisted(() => ({
  backupManagerItems: vi.fn(),
  backupManagerWorkspace: vi.fn(),
  cleanManagerItems: vi.fn(),
  cleanManagerWorkspace: vi.fn(),
  useManagerMeta: vi.fn(),
  useManagerPreview: vi.fn(),
  useManagerProviderCatalog: vi.fn(),
  useManagerStats: vi.fn(),
  useManagerWorkspaces: vi.fn(),
  useStatsDashboard: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  backupManagerItems: mocks.backupManagerItems,
  backupManagerWorkspace: mocks.backupManagerWorkspace,
  cleanManagerItems: mocks.cleanManagerItems,
  cleanManagerWorkspace: mocks.cleanManagerWorkspace,
}));

vi.mock("@/features/manager/queries", () => ({
  useManagerMeta: mocks.useManagerMeta,
  useManagerPreview: mocks.useManagerPreview,
  useManagerProviderCatalog: mocks.useManagerProviderCatalog,
  useManagerStats: mocks.useManagerStats,
  useManagerWorkspaces: mocks.useManagerWorkspaces,
}));

vi.mock("@/features/stats/queries", () => ({
  useStatsDashboard: mocks.useStatsDashboard,
}));

const providerCatalog = {
  providers: [
    {
      provider_id: "codex",
      display_name: "Codex",
      capability_set: {
        scan: true,
        import: true,
        export: true,
        delete: true,
        rename: true,
        resume: true,
        activity_support: {
          hook_events: true,
          runtime_endpoint: true,
          session_activity: true,
        },
      },
      install_state: { is_installed: true },
      filter_tags: ["is_installed"],
      hidden_state: { global: false, workspace: false },
    },
    {
      provider_id: "claude",
      display_name: "Claude",
      capability_set: {
        scan: true,
        import: true,
        export: true,
        delete: true,
        rename: true,
        resume: true,
        activity_support: {
          hook_events: true,
          runtime_endpoint: true,
          session_activity: true,
        },
      },
      install_state: { is_installed: true },
      filter_tags: ["is_installed"],
      hidden_state: { global: false, workspace: false },
    },
  ],
};

const sessions = [
  {
    id: "5:codexalpha",
    provider_id: "codex",
    provider_name: "Codex",
    session_id: "alpha",
    source_path: "/data/alpha.jsonl",
    title: "Alpha session",
    project_dir: "/work/project-one",
    last_active_at: 200,
    size_bytes: 2048,
  },
  {
    id: "6:claudebeta",
    provider_id: "claude",
    provider_name: "Claude",
    session_id: "beta",
    source_path: "/data/beta.jsonl",
    title: "Beta session",
    project_dir: "/work/project-two",
    last_active_at: 100,
    size_bytes: 1024,
  },
];

const workspaces = [
  {
    provider_id: "codex",
    provider_name: "Codex",
    workspace: "/work/project-one",
    session_count: 3,
    total_size_bytes: 4096,
    last_active_at: 200,
  },
  {
    provider_id: "claude",
    provider_name: "Claude",
    workspace: "/work/project-two",
    session_count: 2,
    total_size_bytes: 2048,
    last_active_at: 100,
  },
];

const statsDashboard = {
  generated_at: "2026-08-06T00:00:00Z",
  range_start: null,
  completeness: { status: "complete", incomplete_session_count: 0 },
  overview: {
    total_sessions: 5,
    active_sessions: 3,
    new_sessions: 1,
    total_messages: 100,
    active_session_messages: 80,
    total_size_bytes: 6_144,
    stale_size_bytes: 1_024,
    total_workspaces: 2,
    active_workspaces: 2,
    total_providers: 2,
    active_providers: 2,
    unknown_message_counts: 0,
    unknown_message_timestamps: 0,
    unknown_size_bytes: 0,
    unknown_activity_times: 0,
    unknown_created_times: 0,
  },
  attention: {
    active_7d: { count: 2, size_bytes: 2048 },
    inactive_7_to_30d: { count: 1, size_bytes: 1024 },
    inactive_30_to_90d: { count: 1, size_bytes: 1024 },
    inactive_over_90d: { count: 1, size_bytes: 2048 },
    unknown: { count: 0, size_bytes: 0 },
    large_sessions: { count: 0, size_bytes: 0 },
    short_sessions: { count: 0, size_bytes: 0 },
    large_threshold_bytes: 50_000_000,
    short_max_messages: 3,
  },
  timeline: [],
  providers: [],
  workspaces: [],
  top_sessions: { by_messages: [], by_size: [], recently_active: [] },
  distributions: { session_size: [], message_count: [] },
};

function queryResult<T>(data: T, overrides: Record<string, unknown> = {}) {
  return {
    data,
    error: null,
    isError: false,
    isFetching: false,
    isLoading: false,
    isPending: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

function LocationProbe() {
  const location = useLocation();
  return (
    <output data-testid="location">
      {location.pathname + location.search}
    </output>
  );
}

function TestI18nProvider({ children }: { children: ReactNode }) {
  return (
    <I18nContext.Provider
      value={{
        language: "en",
        languageSetting: "en",
        setLanguageOverride: () => {},
        t: (key, vars) => translate("en", key, vars),
      }}
    >
      {children}
    </I18nContext.Provider>
  );
}

function renderManager(initialEntry = "/manager") {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <TestI18nProvider>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route
              path="*"
              element={
                <>
                  <ManagerPage />
                  <LocationProbe />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </TestI18nProvider>
    </QueryClientProvider>,
  );
}

function rowForText(text: string) {
  const row = screen.getByText(text).closest("[data-manager-row]");
  expect(row).toBeTruthy();
  return row as HTMLElement;
}

function expectSelectionCount(count: number) {
  expect(
    screen.getByRole("group", { name: `${count} selected` }),
  ).toBeTruthy();
}

afterEach(() => cleanup());

beforeEach(() => {
  vi.clearAllMocks();
  useUiStore.setState({ selectedWorkspace: null });
  mocks.useManagerMeta.mockReturnValue(
    queryResult({
      selected_workspace: "/work/project-one",
      settings: { default_backup_dir: "/backups" },
    }),
  );
  mocks.useManagerProviderCatalog.mockReturnValue(queryResult(providerCatalog));
  mocks.useManagerStats.mockReturnValue(
    queryResult({
      selected_agent_count: 2,
      current_workspace_session_count: 2,
      current_workspace_size_bytes: 3072,
      all_workspace_count: 2,
      all_workspace_session_count: 5,
      all_workspace_size_bytes: 6144,
    }),
  );
  mocks.useManagerPreview.mockReturnValue(
    queryResult({ items: sessions, total_count: 2, total_size_bytes: 3072 }),
  );
  mocks.useManagerWorkspaces.mockReturnValue(
    queryResult({ items: workspaces, total_count: 2, total_size_bytes: 6144 }),
  );
  mocks.useStatsDashboard.mockReturnValue({
    dashboard: queryResult(statsDashboard),
    meta: queryResult({
      selected_workspace: "/work/project-one",
      settings: { default_backup_dir: "/backups" },
    }),
    all: false,
  });
  mocks.backupManagerItems.mockResolvedValue({
    success: 1,
    failed: 0,
    files: ["/backups/alpha.json"],
    errors: [],
  });
  mocks.cleanManagerItems.mockResolvedValue({
    success: 1,
    failed: 0,
    freed_bytes: 2048,
    errors: [],
  });
  mocks.backupManagerWorkspace.mockResolvedValue({
    success: 1,
    failed: 0,
    files: ["/backups/workspace.json"],
    errors: [],
  });
  mocks.cleanManagerWorkspace.mockResolvedValue({
    success: 1,
    failed: 0,
    freed_bytes: 4096,
    errors: [],
  });
});

describe("ManagerPage interaction model", () => {
  it("allows deselecting the last provider without falling back to all", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.click(screen.getByRole("button", { name: /Codex/i }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain(
        "providers=codex",
      );
    });

    await user.click(screen.getByRole("button", { name: /Codex/i }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain(
        "providers=none",
      );
    });
    expect(screen.getByText("No providers selected")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /Codex/i }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain(
        "providers=codex",
      );
    });
  });

  it("uses a scope toggle button instead of stat tiles to switch workspace scope", async () => {
    const user = userEvent.setup();
    renderManager();

    expect(
      screen.queryByRole("button", { name: /Current Workspace/i }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /All Workspaces/i })).toBeNull();
    expect(screen.getByText("Current Workspace")).toBeTruthy();
    expect(screen.getByText("Active sessions")).toBeTruthy();
    expect(screen.getByText("Storage used")).toBeTruthy();
    expect(screen.getByText("Inactive 90+ days")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Switch to all" })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Switch to all" }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain("scope=all");
      expect(screen.getByTestId("location").textContent).toContain(
        "view=workspaces",
      );
      expect(screen.getByText("All Workspaces")).toBeTruthy();
    });

    await user.click(screen.getByRole("button", { name: "Switch to current" }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/manager");
    });
  });

  it("keeps row selection separate from opening the session link", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.click(rowForText("Alpha session"));

    expectSelectionCount(1);
    expect(screen.getByTestId("location").textContent).toBe("/manager");

    await user.click(screen.getByRole("link", { name: /Alpha session/i }));

    expect(screen.getByTestId("location").textContent).toBe(
      "/sessions/codex/alpha",
    );
  });

  it("selects only the currently visible search results", async () => {
    const user = userEvent.setup();
    mocks.useManagerPreview.mockImplementation((filter?: { search?: string }) => {
      const items = filter?.search
        ? sessions.filter((item) =>
            item.title?.toLowerCase().includes(filter.search!.toLowerCase()),
          )
        : sessions;
      return queryResult({
        items,
        total_count: items.length,
        total_size_bytes: items.reduce((sum, item) => sum + item.size_bytes, 0),
      });
    });
    renderManager();

    await user.type(
      screen.getByPlaceholderText("Search sessions, providers, or paths"),
      "beta",
    );
    await waitFor(
      () => {
        expect(screen.queryByText("Alpha session")).toBeNull();
      },
      { timeout: 1500 },
    );
    await user.click(screen.getByRole("button", { name: "Select visible" }));

    expectSelectionCount(1);
    expect(rowForText("Beta session").getAttribute("data-selected")).toBe(
      "true",
    );
    expect(screen.queryByText("Alpha session")).toBeNull();
  });

  it("opens a workspace row in its specified provider workspace scope", async () => {
    const user = userEvent.setup();
    renderManager("/manager?view=workspaces&scope=all");

    await user.click(screen.getByRole("link", { name: /project-one/i }));

    const location = screen.getByTestId("location").textContent || "";
    const url = new URL(location, "http://localhost");
    expect(url.pathname).toBe("/manager");
    expect(url.searchParams.get("view")).toBe("sessions");
    expect(url.searchParams.get("workspace")).toBe("/work/project-one");
    expect(url.searchParams.get("providers")).toBe("codex");
  });

  it("uses More only to open a menu and waits for Back up before opening confirmation", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.click(
      screen.getByRole("button", { name: "More actions for Alpha session" }),
    );

    expect(mocks.backupManagerItems).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();

    await user.click(screen.getByRole("menuitem", { name: "Back up" }));

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: "Back up session" }),
    ).toBeTruthy();
    expect(mocks.backupManagerItems).not.toHaveBeenCalled();
  });

  it("shows a blocking destructive confirmation with exact scope before deleting", async () => {
    const user = userEvent.setup();
    let resolveDelete!: (value: {
      success: number;
      failed: number;
      freed_bytes: number;
      errors: string[];
    }) => void;
    mocks.cleanManagerItems.mockReturnValue(
      new Promise((resolve) => {
        resolveDelete = resolve;
      }),
    );
    renderManager();

    await user.click(rowForText("Alpha session"));
    await user.click(screen.getByRole("button", { name: "Delete" }));

    const dialog = screen.getByRole("alertdialog");
    expect(
      within(dialog).getByRole("heading", { name: "Delete session" }),
    ).toBeTruthy();
    expect(within(dialog).getByText("Selected objects")).toBeTruthy();
    expect(within(dialog).getByText("Codex")).toBeTruthy();
    expect(within(dialog).getByText("/work/project-one")).toBeTruthy();
    expect(within(dialog).getByText("2.0 KB")).toBeTruthy();
    expect(within(dialog).getByText(/cannot be undone/i)).toBeTruthy();
    expect(mocks.cleanManagerItems).not.toHaveBeenCalled();

    const confirm = within(dialog).getByRole("button", {
      name: "Delete permanently",
    });
    await user.click(confirm);
    expect(mocks.cleanManagerItems).toHaveBeenCalledTimes(1);
    expect(confirm.hasAttribute("disabled")).toBe(true);
    await user.click(confirm);
    expect(mocks.cleanManagerItems).toHaveBeenCalledTimes(1);

    resolveDelete({ success: 1, failed: 0, freed_bytes: 2048, errors: [] });
    expect(
      await screen.findByRole("heading", { name: "Sessions deleted" }),
    ).toBeTruthy();
  });

  it("distinguishes partial completion and keeps the selection available for retry", async () => {
    const user = userEvent.setup();
    mocks.backupManagerItems.mockResolvedValue({
      success: 1,
      failed: 1,
      files: ["/backups/alpha.json"],
      errors: ["Claude backup failed"],
    });
    renderManager();

    await user.click(screen.getByRole("button", { name: "Select visible" }));
    await user.click(screen.getByRole("button", { name: "Back up" }));
    await user.click(screen.getByRole("button", { name: "Start backup" }));

    expect(
      await screen.findByRole("heading", {
        name: "Some sessions could not be backed up",
      }),
    ).toBeTruthy();
    expect(screen.getByText("Partially completed")).toBeTruthy();
    expect(screen.getByText(/Claude backup failed/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Close" }));
    expectSelectionCount(2);
  });

  it("shows full request failure and keeps the failed selection", async () => {
    const user = userEvent.setup();
    mocks.backupManagerItems.mockRejectedValue(
      new Error("Backup service unavailable"),
    );
    renderManager();

    await user.click(rowForText("Alpha session"));
    await user.click(screen.getByRole("button", { name: "Back up" }));
    await user.click(screen.getByRole("button", { name: "Start backup" }));

    expect(
      await screen.findByRole("heading", { name: "Back up session failed" }),
    ).toBeTruthy();
    expect(screen.getByText("Failed")).toBeTruthy();
    expect(screen.getByText("Backup service unavailable")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Close" }));
    expectSelectionCount(1);
  });

  it("distinguishes initial page loading, result loading, background refresh, and empty scope", () => {
    mocks.useManagerProviderCatalog.mockReturnValue(
      queryResult(undefined, { isLoading: true, isFetching: true }),
    );
    const initialLoading = renderManager();
    expect(screen.getByLabelText("Loading session manager")).toBeTruthy();
    initialLoading.unmount();

    mocks.useManagerProviderCatalog.mockReturnValue(queryResult(providerCatalog));
    mocks.useManagerPreview.mockReturnValue(
      queryResult(undefined, { isLoading: true, isFetching: true }),
    );
    const resultLoading = renderManager();
    expect(screen.getByLabelText("Loading manager results")).toBeTruthy();
    resultLoading.unmount();

    mocks.useManagerPreview.mockReturnValue(
      queryResult(
        { items: sessions, total_count: 2, total_size_bytes: 3072 },
        { isFetching: true },
      ),
    );
    const refreshing = renderManager();
    expect(screen.getByText("Loading page")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Alpha session/i })).toBeTruthy();
    refreshing.unmount();

    mocks.useManagerPreview.mockReturnValue(
      queryResult({ items: [], total_count: 0, total_size_bytes: 0 }),
    );
    renderManager();
    expect(screen.getByText("No sessions in this scope")).toBeTruthy();
  });

  it("explains when current workspace scope has no selected workspace", () => {
    mocks.useManagerMeta.mockReturnValue(
      queryResult({
        selected_workspace: null,
        settings: { default_backup_dir: "/backups" },
      }),
    );

    renderManager();

    expect(screen.getByText("No current workspace")).toBeTruthy();
    expect(
      screen.getByText(
        "Choose a workspace from the app switcher or change the scope to All workspaces.",
      ),
    ).toBeTruthy();
  });

  it("keeps the result list usable when stats fail and retries stats inline", async () => {
    const user = userEvent.setup();
    const refetch = vi.fn();
    mocks.useManagerStats.mockReturnValue(
      queryResult(undefined, {
        error: new Error("Stats failed"),
        isError: true,
        refetch,
      }),
    );

    renderManager();

    expect(screen.getByRole("link", { name: /Alpha session/i })).toBeTruthy();
    expect(screen.getByTestId("manager-stats-error").textContent).toContain(
      "Manager stats failed to load",
    );
    expect(screen.getByTestId("manager-stats-error").textContent).toContain(
      "Stats failed",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("keeps list refresh state independent from stats fetching", () => {
    mocks.useManagerStats.mockReturnValue(
      queryResult(
        {
          selected_agent_count: 2,
          current_workspace_session_count: 2,
          current_workspace_size_bytes: 3072,
          all_workspace_count: 2,
          all_workspace_session_count: 5,
          all_workspace_size_bytes: 6144,
        },
        { isFetching: true },
      ),
    );

    renderManager();

    expect(
      (screen.getByRole("button", { name: "Refresh" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
    expect(screen.queryByText("Loading page")).toBeNull();
  });

  it("distinguishes filter-empty results and retries a failed query", async () => {
    const user = userEvent.setup();
    mocks.useManagerPreview.mockImplementation((filter?: { search?: string }) =>
      queryResult(
        filter?.search
          ? { items: [], total_count: 0, total_size_bytes: 0 }
          : { items: sessions, total_count: 2, total_size_bytes: 3072 },
      ),
    );
    const filtered = renderManager();
    await user.type(
      screen.getByPlaceholderText("Search sessions, providers, or paths"),
      "missing-session",
    );
    expect(
      await screen.findByText(
        "No sessions matched your filters",
        {},
        { timeout: 1500 },
      ),
    ).toBeTruthy();
    filtered.unmount();

    const refetch = vi.fn();
    mocks.useManagerPreview.mockReturnValue(
      queryResult(undefined, {
        error: new Error("Preview failed"),
        isError: true,
        refetch,
      }),
    );
    renderManager();
    expect(screen.getByText("Manager sessions failed to load")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});
