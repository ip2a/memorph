// @vitest-environment jsdom

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nContext } from "@/lib/i18n-context";
import { translate } from "@/lib/i18n-core";
import { SkillsPage } from "./skills-page";

const mocks = vi.hoisted(() => ({
  useSkills: vi.fn(),
  useSkillDetail: vi.fn(),
  useSkillTree: vi.fn(),
  useSkillFilePreview: vi.fn(),
  useSkillAnalysis: vi.fn(),
  install: vi.fn(),
  uninstall: vi.fn(),
}));

vi.mock("@/features/skills/queries", () => ({
  useSkills: mocks.useSkills,
  useSkillDetail: mocks.useSkillDetail,
  useSkillTree: mocks.useSkillTree,
  useSkillFilePreview: mocks.useSkillFilePreview,
  useSkillAnalysis: mocks.useSkillAnalysis,
  useInstallSkill: () => ({
    mutate: mocks.install,
    isPending: false,
    variables: undefined,
    error: null,
  }),
  useUninstallSkill: () => ({
    mutate: mocks.uninstall,
    isPending: false,
    variables: undefined,
    error: null,
  }),
}));

const overview = {
  agents: [
    {
      provider_id: "claude",
      name: "Claude Code",
      skills_dir: "/home/test/.claude/skills",
    },
    {
      provider_id: "codex",
      name: "Codex",
      skills_dir: "/home/test/.codex/skills",
    },
    {
      provider_id: "gemini",
      name: "Gemini CLI",
      skills_dir: "/home/test/.gemini/skills",
    },
  ],
  skills: [
    {
      id: "document-writer",
      name: "Document Writer",
      description: "Writes concise documentation",
      directory: "document-writer",
      fingerprint: "sha256:document-writer",
      conflict: false,
      statistics: { files: 3, bytes: 128, scripts: 1, references: 1, assets: 0, previewable: 3 },
      issues: [],
      installations: [
        {
          provider_id: "claude",
          path: "/home/test/.claude/skills/document-writer",
          managed: false,
          deployment_mode: "external",
          link_valid: true,
          fingerprint: "sha256:document-writer",
          drifted: false,
        },
        {
          provider_id: "gemini",
          path: "/home/test/.gemini/skills/document-writer",
          managed: true,
          deployment_mode: "symlink",
          link_valid: true,
          fingerprint: "sha256:document-writer",
          drifted: false,
        },
      ],
    },
    {
      id: "reviewer",
      name: "Reviewer",
      description: "Reviews code",
      directory: "reviewer",
      fingerprint: "sha256:reviewer",
      conflict: false,
      statistics: { files: 1, bytes: 64, scripts: 0, references: 0, assets: 0, previewable: 1 },
      issues: [],
      installations: [
        {
          provider_id: "codex",
          path: "/home/test/.codex/skills/reviewer",
          managed: true,
          deployment_mode: "copy",
          link_valid: true,
          fingerprint: "sha256:reviewer",
          drifted: false,
        },
      ],
    },
  ],
};

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
  mocks.useSkills.mockReturnValue({
    data: overview,
    error: null,
    isError: false,
    isFetching: false,
    isLoading: false,
    refetch: vi.fn(),
  });
  mocks.useSkillDetail.mockReturnValue({ data: undefined });
  mocks.useSkillTree.mockReturnValue({ data: { assets: [] } });
  mocks.useSkillFilePreview.mockReturnValue({ data: undefined });
  mocks.useSkillAnalysis.mockReturnValue({ data: { skills: [] } });
});

afterEach(() => cleanup());

describe("SkillsPage", () => {
  it("renders the /skills route and filters discovered skills", async () => {
    const user = userEvent.setup();
    renderRoute();

    expect(screen.getByRole("heading", { name: "Skills" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Inspection/i })).toBeTruthy();
    expect(screen.getAllByText("Document Writer").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Reviewer").length).toBeGreaterThan(0);

    await user.type(
      screen.getByRole("textbox", { name: "Search Skills" }),
      "review",
    );
    expect(screen.queryByText("Document Writer")).toBeNull();
    expect(screen.getAllByText("Reviewer").length).toBeGreaterThan(0);
  });

  it("installs into a missing agent and confirms removal of a managed copy", async () => {
    const user = userEvent.setup();
    renderRoute();

    const codex = screen.getByText("Codex").closest("div.rounded-lg");
    expect(codex).toBeTruthy();
    await user.click(
      within(codex as HTMLElement).getByRole("button", { name: "Install" }),
    );
    expect(mocks.install).toHaveBeenCalledWith({
      skill_id: "document-writer",
      provider: "codex",
      source_provider: "claude",
    });

    const gemini = screen.getByText("Gemini CLI").closest("div.rounded-lg");
    expect(gemini).toBeTruthy();
    await user.click(
      within(gemini as HTMLElement).getByRole("button", { name: "Remove" }),
    );
    expect(screen.getByRole("alertdialog")).toBeTruthy();
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
