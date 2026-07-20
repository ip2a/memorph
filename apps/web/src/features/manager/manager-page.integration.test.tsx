// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ManagerPage } from "./manager-page";
import { useUiStore } from "@/stores/ui-store";

const session = {
  id: "5:codexalpha",
  provider_id: "codex",
  provider_name: "Codex",
  session_id: "alpha",
  source_path: "/data/alpha.jsonl",
  title: "Alpha session",
  project_dir: "/work/project-one",
  last_active_at: 200,
  size_bytes: 2048,
};

function jsonResponse(data: unknown) {
  return new Response(JSON.stringify({ ok: true, data }), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function requestPath(input: RequestInfo | URL) {
  return typeof input === "string" ? input : input.toString();
}

function renderManager() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter
        initialEntries={["/manager?scope=all&providers=codex&sort=title"]}
      >
        <Routes>
          <Route path="*" element={<ManagerPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("ManagerPage API workflow", () => {
  it("loads a projected session with route filters and submits the selected backup", async () => {
    useUiStore.setState({ selectedWorkspace: null });
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, init: RequestInit = {}) => {
        const path = requestPath(input);
        if (path === "/api/v1/meta") {
          return jsonResponse({
            selected_workspace: "/work/current",
            settings: { default_backup_dir: "/backups" },
          });
        }
        if (path.startsWith("/api/v1/providers/catalog")) {
          return jsonResponse({
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
            ],
          });
        }
        if (path === "/api/v1/providers") {
          return jsonResponse([
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
          ]);
        }
        if (path === "/api/v1/manager/stats") {
          return jsonResponse({
            selected_agent_count: 1,
            current_workspace_session_count: 1,
            current_workspace_size_bytes: 2048,
            all_workspace_count: 1,
            all_workspace_session_count: 1,
            all_workspace_size_bytes: 2048,
          });
        }
        if (path === "/api/v1/manager/preview") {
          return jsonResponse({
            items: [session],
            total_count: 1,
            total_size_bytes: 2048,
          });
        }
        if (path === "/api/v1/manager/backup") {
          return jsonResponse({
            success: 1,
            failed: 0,
            files: ["/backups/alpha.json"],
            errors: [],
          });
        }
        throw new Error(`Unexpected request: ${init.method ?? "GET"} ${path}`);
      },
    );
    vi.stubGlobal("fetch", fetchMock);

    const user = userEvent.setup();
    renderManager();

    expect(
      await screen.findByRole("link", { name: /Alpha session/i }),
    ).toBeTruthy();

    await waitFor(() => {
      const previewCall = fetchMock.mock.calls.find(
        ([input]) => requestPath(input) === "/api/v1/manager/preview",
      );
      expect(previewCall).toBeTruthy();
      expect(JSON.parse(String(previewCall?.[1]?.body))).toEqual({
        providers: ["codex"],
        sort: "title",
        limit: 20,
        offset: 0,
      });
    });

    await user.click(
      screen.getByText("Alpha session").closest("[data-manager-row]") as HTMLElement,
    );
    await user.click(screen.getByRole("button", { name: "Back up" }));
    await user.click(screen.getByRole("button", { name: "Start backup" }));

    expect(
      await screen.findByRole("heading", { name: "Sessions backed up" }),
    ).toBeTruthy();
    const backupCall = fetchMock.mock.calls.find(
      ([input]) => requestPath(input) === "/api/v1/manager/backup",
    );
    expect(backupCall).toBeTruthy();
    expect(JSON.parse(String(backupCall?.[1]?.body))).toEqual({
      items: [session],
      output_dir: "/backups",
    });
  });
});
