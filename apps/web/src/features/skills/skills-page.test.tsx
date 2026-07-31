// @vitest-environment jsdom

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nContext } from "@/lib/i18n-context";
import { translate } from "@/lib/i18n-core";
import type { SkillCatalogParams } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";
import { SkillsPage } from "./skills-page";

const mocks = vi.hoisted(() => ({
  useSkills: vi.fn(),
  useSkillDetail: vi.fn(),
  useSkillTree: vi.fn(),
  useSkillFilePreview: vi.fn(),
  useSkillStats: vi.fn(),
  useSkillInvocations: vi.fn(),
  useSkillContextSummary: vi.fn(),
  useSkillContext: vi.fn(),
  useSkillHealthSummary: vi.fn(),
  useSkillHealth: vi.fn(),
  useSkillCoverage: vi.fn(),
  useSkillCoverageEvidence: vi.fn(),
  useSkillConflicts: vi.fn(),
  useSkillGraph: vi.fn(),
  useSkillPrune: vi.fn(),
  useExecuteSkillPrune: vi.fn(),
  scan: vi.fn(),
  install: vi.fn(),
  uninstall: vi.fn(),
}));

vi.mock("@/features/skills/queries", () => ({
  useSkills: mocks.useSkills,
  useSkillDetail: mocks.useSkillDetail,
  useSkillTree: mocks.useSkillTree,
  useSkillFilePreview: mocks.useSkillFilePreview,
  useSkillStats: mocks.useSkillStats,
  useSkillInvocations: mocks.useSkillInvocations,
  useSkillContextSummary: mocks.useSkillContextSummary,
  useSkillContext: mocks.useSkillContext,
  useSkillHealthSummary: mocks.useSkillHealthSummary,
  useSkillHealth: mocks.useSkillHealth,
  useSkillCoverage: mocks.useSkillCoverage,
  useSkillCoverageEvidence: mocks.useSkillCoverageEvidence,
  useSkillConflicts: mocks.useSkillConflicts,
  useSkillGraph: mocks.useSkillGraph,
  useSkillPrune: mocks.useSkillPrune,
  useExecuteSkillPrune: mocks.useExecuteSkillPrune,
  useScanSkills: () => ({
    mutate: mocks.scan,
    isPending: false,
    error: null,
  }),
  useInstallSkill: () => ({
    mutate: mocks.install,
    isPending: false,
    error: null,
  }),
  useUninstallSkill: () => ({
    mutate: mocks.uninstall,
    isPending: false,
    error: null,
  }),
}));

const items = [
  {
    id: "skill:document-writer",
    source_id: "document-writer",
    name: "Document Writer",
    description: "Writes concise documentation",
    bundle_hash: "sha256:document-writer",
    file_count: 3,
    total_bytes: 128,
    missing: false,
    updated_at_ms: 1,
    installations: [
      {
        provider_id: "claude",
        scope_kind: "global",
        install_path: "/home/test/.claude/skills/document-writer",
        install_kind: "directory",
        link_status: "not-applicable",
        status: "active",
      },
      {
        provider_id: "gemini",
        scope_kind: "global",
        install_path: "/home/test/.gemini/skills/document-writer",
        install_kind: "symlink",
        link_status: "valid",
        status: "active",
      },
    ],
  },
  {
    id: "skill:reviewer",
    source_id: "reviewer",
    name: "Reviewer",
    description: "Reviews code",
    bundle_hash: "sha256:reviewer",
    file_count: 1,
    total_bytes: 64,
    missing: false,
    updated_at_ms: 1,
    installations: [
      {
        provider_id: "codex",
        scope_kind: "global",
        install_path: "/home/test/.codex/skills/reviewer",
        install_kind: "managed-copy",
        link_status: "not-applicable",
        status: "active",
      },
    ],
  },
];

function renderRoute() {
  return render(
    <I18nContext.Provider
      value={{
        language: "en",
        languageSetting: "en",
        setLanguageOverride: vi.fn(),
        t: (key, vars) => translate("en", key, vars),
      }}
    >
      <MemoryRouter initialEntries={["/skills"]}>
        <Routes>
          <Route path="/skills" element={<SkillsPage />} />
        </Routes>
      </MemoryRouter>
    </I18nContext.Provider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useUiStore.setState({ selectedWorkspace: null });
  mocks.useSkills.mockImplementation((params: SkillCatalogParams) => {
    const filtered = params.query
      ? items.filter((item) =>
          item.name.toLowerCase().includes(params.query!.toLowerCase()),
        )
      : items;
    return {
      data: {
        items: filtered,
        page: 1,
        page_size: 50,
        total: filtered.length,
        providers: ["claude", "codex", "gemini"],
        completeness: { status: "partial" },
      },
      error: null,
      isError: false,
      isFetching: false,
      isLoading: false,
      refetch: vi.fn(),
    };
  });
  mocks.useSkillDetail.mockReturnValue({ data: undefined });
  mocks.useSkillTree.mockReturnValue({
    data: { assets: [] },
    isLoading: false,
  });
  mocks.useSkillFilePreview.mockReturnValue({
    data: undefined,
    isLoading: false,
  });
  mocks.useSkillStats.mockReturnValue({
    summary: { data: undefined, isLoading: false },
    daily: { data: [] },
    ranking: { data: [] },
    breakdown: { data: { providers: [], workspaces: [] } },
  });
  mocks.useSkillInvocations.mockReturnValue({ data: undefined });
  mocks.useSkillContextSummary.mockReturnValue({ data: undefined });
  mocks.useSkillContext.mockReturnValue({ data: undefined });
  mocks.useSkillHealthSummary.mockReturnValue({ data: undefined });
  mocks.useSkillHealth.mockReturnValue({ data: undefined });
  mocks.useSkillCoverageEvidence.mockReturnValue({ data: { items: [] } });
  mocks.useSkillCoverage.mockReturnValue({
    data: undefined,
    isLoading: false,
    isError: false,
  });
  mocks.useSkillGraph.mockReturnValue({
    data: { days: [], total_invocations: 0, max_count: 0 },
    isError: false,
  });
  mocks.useSkillPrune.mockReturnValue({ data: { items: [] } });
  mocks.useExecuteSkillPrune.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  });
  mocks.useSkillConflicts.mockReturnValue({
    data: [],
    isLoading: false,
    isError: false,
  });
});

afterEach(() => cleanup());

describe("SkillsPage", () => {
  it("starts one incremental scan for an uninitialized catalog", async () => {
    mocks.useSkills.mockReturnValue({
      data: {
        items: [],
        page: 1,
        page_size: 50,
        total: 0,
        providers: [],
        completeness: { status: "unknown" },
        needs_scan: true,
      },
      error: null,
      isError: false,
      isFetching: false,
      isLoading: false,
      refetch: vi.fn(),
    });

    renderRoute();

    await waitFor(() =>
      expect(mocks.scan).toHaveBeenCalledWith({
        mode: "incremental",
        workspace: undefined,
      }),
    );
    expect(mocks.scan).toHaveBeenCalledTimes(1);
  });

  it("keeps the page visible while only the catalog list is loading", () => {
    mocks.useSkills.mockReturnValue({
      data: undefined,
      error: null,
      isError: false,
      isFetching: true,
      isLoading: true,
      refetch: vi.fn(),
    });

    renderRoute();

    expect(screen.getByRole("heading", { name: "Skills" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeTruthy();
  });

  it("renders the SQLite catalog and sends search filters to the query", async () => {
    const user = userEvent.setup();
    renderRoute();
    expect(screen.getByRole("heading", { name: "Skills" })).toBeTruthy();
    expect(screen.getAllByText("Document Writer").length).toBeGreaterThan(0);
    await user.type(
      screen.getByRole("textbox", { name: "Search Skills" }),
      "review",
    );
    expect(mocks.useSkills).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "review" }),
    );
    expect(screen.queryByText("Document Writer")).toBeNull();
    expect(screen.getAllByText("Reviewer").length).toBeGreaterThan(0);
  });

  it("scans global and current project roots", async () => {
    useUiStore.setState({ selectedWorkspace: "/work/demo" });
    const user = userEvent.setup();
    renderRoute();

    await user.click(screen.getByRole("button", { name: "Refresh" }));

    expect(mocks.scan).toHaveBeenCalledWith(
      { mode: "incremental", workspace: "/work/demo" },
      expect.any(Object),
    );
  });

  it("stores custom stats dates in the URL-backed query", async () => {
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByRole("tab", { name: "Custom" }));
    fireEvent.change(screen.getByLabelText("Start date"), {
      target: { value: "2026-07-01" },
    });
    fireEvent.change(screen.getByLabelText("End date"), {
      target: { value: "2026-07-22" },
    });
    expect(mocks.useSkillStats).toHaveBeenLastCalledWith(
      expect.objectContaining({
        from: "2026-07-01",
        to: "2026-07-22",
      }),
    );
  });

  it("drills into a day on the activity heatmap", async () => {
    mocks.useSkillGraph.mockReturnValue({
      data: {
        days: [
          {
            date: "2026-07-22",
            invocations: 3,
            sessions: 2,
            active_skills: 1,
            level: 2,
          },
        ],
        total_invocations: 3,
        max_count: 3,
      },
      isError: false,
    });
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    expect(mocks.useSkillGraph).toHaveBeenLastCalledWith(
      expect.not.objectContaining({ workspace: expect.anything() }),
    );
    await user.click(screen.getByRole("button", { name: /2026-07-22/ }));
    expect(screen.getByRole("dialog").textContent).toContain(
      "2026-07-22 Skill Invocations",
    );
    expect(mocks.useSkillStats).toHaveBeenLastCalledWith(
      expect.objectContaining({ from: "2026-07-22", to: "2026-07-22" }),
    );
  });

  it("shows target coverage in the coverage tab", async () => {
    mocks.useSkillCoverage.mockReturnValue({
      data: {
        skill_id: "document-writer",
        covered: 1,
        total: 1,
        percent: 100,
        completeness_status: "complete",
        targets: [
          {
            target_kind: "script",
            target_key: "scripts/run.sh",
            target_path: "scripts/run.sh",
            confidence: "high",
            observations: 2,
          },
        ],
      },
      isLoading: false,
      isError: false,
    });
    mocks.useSkillTree.mockReturnValue({
      data: {
        assets: [
          {
            path: "scripts/run.sh",
            category: "script",
            bytes: 12,
            previewable: true,
            entry: false,
          },
        ],
      },
      isLoading: false,
    });
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("tab", { name: "Coverage and Conflicts" }));
    await user.click(screen.getByRole("tab", { name: "Scripts" }));
    expect(screen.getByText("2 · high")).toBeTruthy();
  });

  it("opens paged coverage evidence with the provider session route", async () => {
    mocks.useSkillCoverage.mockReturnValue({
      data: {
        skill_id: "skill:document-writer",
        covered: 1,
        total: 1,
        percent: 100,
        completeness_status: "complete",
        targets: [
          {
            target_kind: "section",
            target_key: "intro",
            section_title: "Introduction",
            confidence: "high",
            observations: 1,
          },
        ],
      },
      isLoading: false,
      isError: false,
    });
    mocks.useSkillCoverageEvidence.mockReturnValue({
      data: {
        items: [
          {
            invocation_id: "invocation-1",
            session_id: "session-1",
            provider_id: "codex",
            observed_at_ms: 1,
            match_kind: "section-anchor",
            confidence: "high",
          },
        ],
        page: 1,
        page_size: 20,
        total: 1,
      },
    });
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("tab", { name: "Coverage and Conflicts" }));
    await user.click(screen.getByText("Introduction"));
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain("Coverage Evidence");
    expect(
      within(dialog)
        .getByRole("link", { name: /Open session/ })
        .getAttribute("href"),
    ).toBe("/sessions/codex/session-1");
  });

  it("installs into a missing provider and safely removes a managed installation", async () => {
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("tab", { name: "Installations" }));
    const codex = screen
      .getAllByText("codex")
      .map((element) => element.closest("div.rounded-lg"))
      .find(Boolean);
    await user.click(
      within(codex as HTMLElement).getByRole("button", { name: "Install" }),
    );
    expect(mocks.install).toHaveBeenCalledWith({
      skill_id: "document-writer",
      provider: "codex",
      source_provider: "claude",
    });
    const gemini = screen
      .getAllByText("gemini")
      .map((element) => element.closest("div.rounded-lg"))
      .find(Boolean);
    await user.click(
      within(gemini as HTMLElement).getByRole("button", { name: "Remove" }),
    );
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Remove",
      }),
    );
    expect(mocks.uninstall).toHaveBeenCalledWith({
      skill_id: "document-writer",
      provider: "gemini",
    });
  });
});
