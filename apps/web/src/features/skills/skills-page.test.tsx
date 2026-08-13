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
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
  useUpdateSkillFile: vi.fn(),
  analyze: vi.fn(),
  scan: vi.fn(),
  install: vi.fn(),
  uninstall: vi.fn(),
  delete: vi.fn(),
  disable: vi.fn(),
  consolidate: vi.fn(),
  removeSymlinks: vi.fn(),
  getMeta: vi.fn(),
  updateSettings: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    getMeta: mocks.getMeta,
    updateSettings: mocks.updateSettings,
  };
});

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
  useUpdateSkillFile: () => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  }),
  useAnalyzeSkills: () => ({
    mutate: mocks.analyze,
    isPending: false,
    error: null,
  }),
  useCurrentSkillAnalysis: () => ({ data: null, isLoading: false }),
  useSkillAnalysisOperation: () => ({ data: null, isLoading: false }),
  useScanSkills: () => ({
    mutate: mocks.scan,
    mutateAsync: mocks.scan,
    isPending: false,
    isSuccess: false,
    error: null,
    variables: undefined,
    reset: vi.fn(),
  }),
  useInstallSkill: () => ({
    mutate: mocks.install,
    mutateAsync: mocks.install,
    isPending: false,
    isSuccess: false,
    error: null,
    variables: undefined,
    reset: vi.fn(),
  }),
  useUninstallSkill: () => ({
    mutate: mocks.uninstall,
    mutateAsync: mocks.uninstall,
    isPending: false,
    isSuccess: false,
    error: null,
    variables: undefined,
    reset: vi.fn(),
  }),
  useDeleteSkill: () => ({
    mutate: mocks.delete,
    mutateAsync: mocks.delete,
    isPending: false,
    isSuccess: false,
    error: null,
    variables: undefined,
    reset: vi.fn(),
  }),
  useDisableSkill: () => ({
    mutate: mocks.disable,
    isPending: false,
    error: null,
    variables: undefined,
  }),
  useConsolidateSkill: () => ({
    mutate: mocks.consolidate,
    isPending: false,
    error: null,
    variables: undefined,
  }),
  useRemoveSymlinksSkill: () => ({
    mutate: mocks.removeSymlinks,
    isPending: false,
    error: null,
    variables: undefined,
  }),
  useDeleteSkillInstallation: () => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
    variables: undefined,
  }),
  useDisabledSkills: () => ({ data: { items: [] }, isLoading: false }),
  useSkillGroupInstallations: () => ({
    data: {
      installations: [
        {
          used_by: "claude",
          path: "/home/test/.claude/skills/document-writer",
          managed: false,
          deployment_mode: "external",
          link_valid: true,
          fingerprint: "sha256:a",
          drifted: false,
          scope_kind: "global",
          link_status: "not-applicable",
        },
        {
          used_by: "gemini",
          path: "/home/test/.gemini/skills/document-writer",
          managed: true,
          deployment_mode: "symlink",
          link_valid: true,
          fingerprint: "sha256:a",
          drifted: false,
          symlink_target: "/home/test/.claude/skills/document-writer",
          scope_kind: "global",
          link_status: "valid",
        },
      ],
    },
    isLoading: false,
  }),
  useEnableSkill: () => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
    variables: undefined,
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
    tags: ["documentation"],
    used_by: ["claude", "gemini"],
    installations: [
      {
        used_by: "claude",
        scope_kind: "global",
        install_path: "/home/test/.claude/skills/document-writer",
        install_kind: "directory",
        link_status: "not-applicable",
        status: "active",
      },
      {
        used_by: "gemini",
        scope_kind: "global",
        install_path: "/home/test/.gemini/skills/document-writer",
        install_kind: "symlink",
        symlink_target: "/home/test/.claude/skills/document-writer",
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
    tags: ["review"],
    used_by: ["codex"],
    installations: [
      {
        used_by: "codex",
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
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
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
      </I18nContext.Provider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useUiStore.setState({ selectedWorkspace: null });
  mocks.getMeta.mockResolvedValue({
    version: "0.1.32",
    settings: {
      skills_catalog_page_size: 50,
      sessions_per_provider: 12,
      language: "en",
      home_session_layout: "tabs",
      agent_order: [],
      primary_agents: [],
    },
  });
  mocks.updateSettings.mockResolvedValue({
    skills_catalog_page_size: 25,
    sessions_per_provider: 12,
    language: "en",
  });
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
        used_by: ["claude", "codex", "gemini"],
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
  mocks.useSkillConflicts.mockReturnValue({
    data: [],
    isLoading: false,
    isError: false,
  });
});

afterEach(() => cleanup());

describe("SkillsPage", () => {
  it("starts one incremental scan after the catalog loads", async () => {
    mocks.useSkills.mockReturnValue({
      data: {
        items: [],
        page: 1,
        page_size: 50,
        total: 0,
        used_by: [],
        completeness: { status: "unknown" },
        needs_scan: false,
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
    expect(screen.getByRole("button", { name: "Refresh List" })).toBeTruthy();
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

  it("filters the catalog by which Agent uses a Skill", async () => {
    const user = userEvent.setup();
    renderRoute();

    await user.click(screen.getByRole("button", { name: "Filter" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Used by Agents" }),
      "codex",
    );
    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(mocks.useSkills).toHaveBeenLastCalledWith(
      expect.objectContaining({ used_by: "codex" }),
    );
  });

  it("paginates the catalog in pages of fifty", async () => {
    const catalogItems = Array.from({ length: 51 }, (_, index) => ({
      id: `skill-${index}`,
      source_id: `skill-${index}`,
      name: `Skill ${index}`,
      description: null,
      version: null,
      author: null,
      bundle_hash: `hash-${index}`,
      file_count: 1,
      total_bytes: 100,
      missing: false,
      updated_at_ms: index,
      tags: [],
      used_by: ["codex"],
      installations: [],
    }));
    mocks.useSkills.mockImplementation((params: SkillCatalogParams) => {
      const currentPage = params.page ?? 1;
      const pageSize = params.pageSize ?? 50;
      const start = (currentPage - 1) * pageSize;
      const slice = catalogItems.slice(start, start + pageSize);
      return {
        data: {
          items: slice,
          page: currentPage,
          page_size: pageSize,
          total: catalogItems.length,
          used_by: ["codex"],
          completeness: { status: "complete" },
        },
        error: null,
        isError: false,
        isFetching: false,
        isLoading: false,
        refetch: vi.fn(),
      };
    });

    const user = userEvent.setup();
    renderRoute();

    expect(screen.getByText("1–50/51 · 1/2")).toBeTruthy();
    expect(screen.getByText("Skill 0")).toBeTruthy();
    expect(screen.queryByText("Skill 50")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Next" }));

    expect(mocks.useSkills).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2, pageSize: 50 }),
    );
    expect(screen.getByText("51/51 · 2/2")).toBeTruthy();
    expect(screen.getByText("Skill 50")).toBeTruthy();
  });

  it("persists page size from the filter dialog", async () => {
    const user = userEvent.setup();
    renderRoute();

    await user.click(screen.getByRole("button", { name: "Filter" }));
    await user.click(screen.getByRole("button", { name: "20 per page" }));
    await user.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => expect(mocks.updateSettings).toHaveBeenCalled());
    expect(mocks.updateSettings.mock.calls[0]?.[0]).toEqual(
      expect.objectContaining({ skills_catalog_page_size: 20 }),
    );
  });

  it("uses the catalog page size from settings", async () => {
    mocks.getMeta.mockResolvedValue({
      version: "0.1.32",
      settings: {
        skills_catalog_page_size: 25,
        sessions_per_provider: 12,
        language: "en",
      },
    });
    renderRoute();
    await waitFor(() =>
      expect(mocks.useSkills).toHaveBeenCalledWith(
        expect.objectContaining({ pageSize: 25 }),
      ),
    );
  });

  it("scans global and current project roots", async () => {
    useUiStore.setState({ selectedWorkspace: "/work/demo" });
    const user = userEvent.setup();
    renderRoute();

    await user.click(screen.getByRole("button", { name: "Refresh List" }));

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
    await user.click(screen.getByRole("button", { name: "Apply" }));
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

  it("installs for a missing Agent and safely removes a managed installation", async () => {
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("tab", { name: "Installations" }));
    const codex = screen
      .getAllByText("codex")
      .map((element) => element.closest("div.rounded-lg"))
      .find(Boolean);
    await user.click(
      within(codex as HTMLElement).getByRole("button", {
        name: "Add symlink for agent",
      }),
    );
    expect(mocks.install).toHaveBeenCalledWith(
      expect.objectContaining({
        skill_id: "document-writer",
        used_by: "codex",
        source_used_by: "claude",
        scope_kind: "global",
      }),
      expect.any(Object),
    );
    const gemini = screen
      .getAllByText("gemini")
      .map((element) => element.closest("div.rounded-lg"))
      .filter(Boolean)
      .find((div) =>
        within(div as HTMLElement).queryByRole("button", {
          name: "Remove agent symlink",
        }),
      );
    await user.click(
      within(gemini as HTMLElement).getByRole("button", {
        name: "Remove agent symlink",
      }),
    );
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Remove",
      }),
    );
    expect(mocks.uninstall).toHaveBeenCalledWith(
      expect.objectContaining({
        skill_id: "document-writer",
        used_by: "gemini",
        scope_kind: "global",
      }),
      expect.any(Object),
    );
  });

  it("disables the selected skill after confirming", async () => {
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("button", { name: "Disable" }));
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Disable",
      }),
    );
    expect(mocks.disable).toHaveBeenCalledWith(
      "skill:document-writer",
      expect.anything(),
    );
  });

  it("removes symlinks from the consolidate tab", async () => {
    const user = userEvent.setup();
    renderRoute();
    await user.click(screen.getByText("Document Writer"));
    await user.click(screen.getByRole("tab", { name: "Consolidate" }));
    await user.click(screen.getByRole("button", { name: "Remove Symlinks" }));
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Remove Symlinks",
      }),
    );
    expect(mocks.removeSymlinks).toHaveBeenCalledWith(
      "skill:document-writer",
      expect.anything(),
    );
  });
});
