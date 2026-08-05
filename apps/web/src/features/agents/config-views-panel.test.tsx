// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentManagementEntry, ProviderConfigView, ProviderSettingItem } from "@/lib/types";
import { ConfigViewsBlock } from "./config-views-panel";

const useProviderConfigView = vi.hoisted(() => vi.fn());
const useRemoveProviderConfigEntry = vi.hoisted(() => vi.fn());
vi.mock("@/features/agents/queries", () => ({
  useProviderConfigView: (...args: unknown[]) => useProviderConfigView(...args),
  useRemoveProviderConfigEntry: (...args: unknown[]) => useRemoveProviderConfigEntry(...args),
}));

vi.mock("@/lib/i18n-context", () => ({
  useI18n: () => ({
    t: (key: string) => ({
      configuration: "Configuration",
      configurationDescription: "Read-only inspection",
      missing: "missing",
      no: "no",
      ok: "OK",
      provider: "Provider",
      remove: "Remove",
      removeMcpConfiguration: "Remove MCP configuration",
      scope: "Scope",
      source: "Source",
      warning: "Warning",
      danger: "Danger",
      muted: "Muted",
      readFrom: "Read from",
      yes: "yes",
    })[key] ?? key,
  }),
}));

afterEach(() => {
  cleanup();
  useProviderConfigView.mockReset();
  useRemoveProviderConfigEntry.mockReset();
});

function makeProvider(settings: ProviderSettingItem[]): AgentManagementEntry {
  return { provider_id: "claude", settings } as unknown as AgentManagementEntry;
}

function viewSetting(id: string, title: string): ProviderSettingItem {
  return { id, title, description: `${title} view`, scope: "global", kind: "view" };
}

const MCP_VIEW: ProviderConfigView = {
  provider_id: "claude",
  view_id: "view_mcp",
  title: "MCP servers",
  sources: [{ path: "~/.claude.json", scope: "user", exists: true }],
  sections: [{ label: "openpage", rows: [{ label: "Type", value: "stdio" }] }],
  issues: [],
};

describe("ConfigViewsBlock", () => {
  it("renders nothing when the provider has no view settings", () => {
    useProviderConfigView.mockReturnValue({ data: undefined, isLoading: false });
    const { container } = render(
      <ConfigViewsBlock
        provider={makeProvider([
          { id: "show_subagents", title: "Show subagents", description: "", scope: "global", kind: "toggle" },
        ])}
      />,
    );
    expect(container.querySelector("[data-config-views]")).toBeNull();
  });

  it("loads and renders view content directly when the tab mounts", async () => {
    useProviderConfigView.mockReturnValue({ data: MCP_VIEW, isLoading: false, error: undefined });

    render(<ConfigViewsBlock provider={makeProvider([viewSetting("view_mcp", "MCP servers")])} />);

    expect(useProviderConfigView).toHaveBeenCalledWith("claude", "view_mcp", true);
    expect(await screen.findByText(/Read from/)).toBeTruthy();
    expect(screen.getByText("openpage")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /MCP servers/ })).toBeNull();
  });

  it("shows removal only for explicitly removable MCP entries and confirms their context", () => {
    const mutate = vi.fn();
    useRemoveProviderConfigEntry.mockReturnValue({ mutate, isPending: false, error: null, reset: vi.fn() });
    useProviderConfigView.mockReturnValue({
      data: {
        ...MCP_VIEW,
        entries: [{ entry_id: "demo", name: "openpage", scope: "User", source: "~/.claude.json", fingerprint: "fp-1", removable: true }],
      },
      isLoading: false,
      error: undefined,
    });

    render(<ConfigViewsBlock provider={makeProvider([viewSetting("view_mcp", "MCP servers")])} />);

    fireEvent.click(screen.getByRole("button", { name: "Remove MCP configuration: openpage" }));
    expect(screen.getByText("Provider:")).toBeTruthy();
    expect(screen.getByText(/MCP:/)).toBeTruthy();
    expect(screen.getByText("User")).toBeTruthy();
    expect(screen.getByText("~/.claude.json")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(mutate).toHaveBeenCalledWith(
      { provider: "claude", viewId: "view_mcp", entryId: "demo", expectedFingerprint: "fp-1" },
      expect.any(Object),
    );
  });

  it("does not expose removal for entries without backend removal metadata", () => {
    useProviderConfigView.mockReturnValue({
      data: { ...MCP_VIEW, entries: [{ entry_id: "demo", name: "openpage", fingerprint: "fp-1" }] },
      isLoading: false,
      error: undefined,
    });

    render(<ConfigViewsBlock provider={makeProvider([viewSetting("view_mcp", "MCP servers")])} />);

    expect(screen.queryByRole("button", { name: /Remove MCP configuration/ })).toBeNull();
  });
});
