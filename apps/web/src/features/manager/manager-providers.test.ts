import { describe, expect, it } from "vitest";
import {
  managerProviderCandidates,
  managerProviderOptions,
} from "./manager-providers";
import type { ProviderCapabilities, ProviderCatalogEntry } from "@/lib/types";

function defaultCapabilities(overrides: Partial<ProviderCapabilities> = {}): ProviderCapabilities {
  return {
    scan: true,
    import: true,
    export: true,
    delete: true,
    rename: true,
    resume: true,
    scan_strategy: "indexed",
    page_strategy: "indexed_page",
    storage_shape: "jsonl",
    turn_quality: "exact",
    import_fidelity: {},
    export_fidelity: {},
    resume_quality: "native",
    write_risk: {
      level: "low",
      multiple_files: false,
      sqlite: false,
      sidecar_files: false,
      index_repair: false,
    },
    backup_support: {
      before_write: true,
      restore: true,
      sync_only: false,
    },
    activity_support: {
      hook_events: true,
      runtime_endpoint: true,
      session_activity: true,
    },
    ...overrides,
  };
}

function entry(
  providerId: string,
  overrides: Partial<ProviderCatalogEntry> = {},
): ProviderCatalogEntry {
  return {
    provider_id: providerId,
    display_name: providerId,
    capability_set: defaultCapabilities(),
    install_state: { is_installed: true },
    filter_tags: ["is_installed"],
    hidden_state: { global: false, workspace: false },
    ...overrides,
  };
}

describe("managerProviderCandidates", () => {
  it("keeps installed, scannable, non-hidden providers in catalog order", () => {
    const catalog = [
      entry("codex"),
      entry("claude", { hidden_state: { global: true } }),
      entry("cursor", {
        install_state: { is_installed: false },
        filter_tags: [],
      }),
      entry("opencode", {
        capability_set: defaultCapabilities({ scan: false }),
      }),
    ];

    expect(managerProviderCandidates(catalog).map((item) => item.provider_id)).toEqual([
      "codex",
    ]);
  });

  it("maps candidates to manager provider options", () => {
    const catalog = [
      entry("codex", { display_name: "Codex" }),
      entry("claude", { display_name: "Claude" }),
    ];

    expect(managerProviderOptions(catalog)).toEqual([
      { id: "codex", name: "Codex" },
      { id: "claude", name: "Claude" },
    ]);
  });
});
