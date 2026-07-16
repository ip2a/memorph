// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
import { useUiStore } from "@/stores/ui-store";

const mocks = vi.hoisted(() => ({
  backupManagerItems: vi.fn(),
  backupManagerWorkspace: vi.fn(),
  cleanManagerItems: vi.fn(),
  cleanManagerWorkspace: vi.fn(),
  useManagerMeta: vi.fn(),
  useManagerPreview: vi.fn(),
  useManagerProviders: vi.fn(),
  useManagerStats: vi.fn(),
  useManagerWorkspaces: vi.fn(),
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
  useManagerProviders: mocks.useManagerProviders,
  useManagerStats: mocks.useManagerStats,
  useManagerWorkspaces: mocks.useManagerWorkspaces,
}));

const providers = [
  {
    id: "codex",
    name: "Codex",
    scan: true,
    import: true,
    export: true,
    delete: true,
    rename: true,
    resume: true,
  },
  {
    id: "claude",
    name: "Claude",
    scan: true,
    import: true,
    export: true,
    delete: true,
    rename: true,
    resume: true,
  },
];

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

function renderManager(initialEntry = "/manager") {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
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
    </QueryClientProvider>,
  );
}

function rowForText(text: string) {
  const row = screen.getByText(text).closest("[data-manager-row]");
  expect(row).toBeTruthy();
  return row as HTMLElement;
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
  mocks.useManagerProviders.mockReturnValue(queryResult(providers));
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
  it("makes All providers explicit and closes the final explicit selection back to all", async () => {
    const user = userEvent.setup();
    renderManager();

    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /Codex/i }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain(
        "providers=codex",
      );
    });

    await user.click(screen.getByRole("button", { name: /All providers/i }));
    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/manager");
    });
    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /Codex/i }));
    await user.click(screen.getByRole("button", { name: /Codex/i }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/manager");
    });
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();
  });

  it("keeps row selection separate from opening the session link", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.click(rowForText("Alpha session"));

    expect(screen.getByText("1 selected")).toBeTruthy();
    expect(screen.getByTestId("location").textContent).toBe("/manager");

    await user.click(screen.getByRole("link", { name: /Alpha session/i }));

    expect(screen.getByTestId("location").textContent).toBe(
      "/sessions/codex/alpha",
    );
  });

  it("selects only the currently visible search results", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.type(
      screen.getByPlaceholderText("Search sessions, providers, or paths"),
      "beta",
    );
    await user.click(screen.getByRole("button", { name: "Select visible" }));

    expect(screen.getByText("1 selected")).toBeTruthy();
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
    expect(screen.getByText("2 selected")).toBeTruthy();
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
    expect(screen.getByText("1 selected")).toBeTruthy();
  });

  it("distinguishes initial page loading, result loading, background refresh, and empty scope", () => {
    mocks.useManagerProviders.mockReturnValue(
      queryResult(undefined, { isLoading: true, isFetching: true }),
    );
    const initialLoading = renderManager();
    expect(screen.getByLabelText("Loading session manager")).toBeTruthy();
    initialLoading.unmount();

    mocks.useManagerProviders.mockReturnValue(queryResult(providers));
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
    expect(screen.getByText("Refreshing results")).toBeTruthy();
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

  it("distinguishes filter-empty results and retries a failed query", async () => {
    const user = userEvent.setup();
    const filtered = renderManager();
    await user.type(
      screen.getByPlaceholderText("Search sessions, providers, or paths"),
      "missing-session",
    );
    expect(screen.getByText("No sessions matched your filters")).toBeTruthy();
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
