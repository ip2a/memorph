// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentManagementEntry, ProviderConfigView, ProviderSettingItem } from "@/lib/types";
import { ConfigViewsBlock } from "./config-views-panel";

const useProviderConfigView = vi.hoisted(() => vi.fn());
vi.mock("@/features/agents/queries", () => ({
  useProviderConfigView: (...args: unknown[]) => useProviderConfigView(...args),
}));

afterEach(() => {
  cleanup();
  useProviderConfigView.mockReset();
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

  it("declares a panel per view and only fetches content once expanded", async () => {
    // The hook mirrors its `enabled` argument: no data while closed, the view once open.
    useProviderConfigView.mockImplementation((_provider, _viewId, enabled) =>
      enabled ? { data: MCP_VIEW, isLoading: false } : { data: undefined, isLoading: false },
    );

    render(<ConfigViewsBlock provider={makeProvider([viewSetting("view_mcp", "MCP servers")])} />);

    // Declared, but content is not fetched yet (panel closed).
    const button = await screen.findByRole("button", { name: /MCP servers/ });
    expect(screen.queryByText(/Read from/)).toBeNull();
    expect(useProviderConfigView).toHaveBeenLastCalledWith("claude", "view_mcp", false);

    // Expanding flips the gate and renders the lazily-loaded content.
    fireEvent.click(button);
    expect(useProviderConfigView).toHaveBeenLastCalledWith("claude", "view_mcp", true);
    expect(await screen.findByText(/Read from/)).toBeTruthy();
    expect(screen.getByText("openpage")).toBeTruthy();
  });
});
