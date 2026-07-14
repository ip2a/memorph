// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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

function queryResult<T>(data: T) {
  return {
    data,
    error: null,
    isError: false,
    isFetching: false,
    isLoading: false,
    isPending: false,
    refetch: vi.fn(),
  };
}

function LocationProbe() {
  const location = useLocation();
  return <output data-testid="location">{location.pathname + location.search}</output>;
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
});

describe("ManagerPage interaction model", () => {
  it("makes All providers explicit and closes the final explicit selection back to all", async () => {
    const user = userEvent.setup();
    renderManager();

    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /All providers/i }));
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Codex" }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toContain("providers=codex");
    });
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Codex" }).getAttribute("aria-checked"),
    ).toBe("true");

    await user.click(screen.getByRole("menuitemcheckbox", { name: "All providers" }));
    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/manager");
    });
    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /All providers/i }));
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Codex" }));
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Codex" }));

    await waitFor(() => {
      expect(screen.getByTestId("location").textContent).toBe("/manager");
    });
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: /All providers/i })).toBeTruthy();
  });

  it("keeps checkbox selection separate from opening the session row", async () => {
    const user = userEvent.setup();
    renderManager();

    await user.click(screen.getByRole("checkbox", { name: "Select Alpha session" }));

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
    expect(
      screen.getByRole("checkbox", { name: "Select Beta session" }).getAttribute("data-state"),
    ).toBe("checked");
    expect(screen.queryByRole("checkbox", { name: "Select Alpha session" })).toBeNull();
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
    expect(screen.getByRole("heading", { name: "Back up session" })).toBeTruthy();
    expect(mocks.backupManagerItems).not.toHaveBeenCalled();
  });
});
