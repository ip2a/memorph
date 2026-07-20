import { describe, expect, it } from "vitest";
import {
  providerLogoFallbackInitial,
  resolveProviderLogoAssetId,
} from "@/components/shared/provider-logo-map";

describe("resolveProviderLogoAssetId", () => {
  it("maps memorph provider ids to logo assets", () => {
    expect(resolveProviderLogoAssetId("claude")).toBe("claude");
    expect(resolveProviderLogoAssetId("codex")).toBe("codex");
    expect(resolveProviderLogoAssetId("kimi")).toBe("kimi-for-coding");
    expect(resolveProviderLogoAssetId("copilot")).toBe("github-copilot");
  });

  it("accepts direct asset ids", () => {
    expect(resolveProviderLogoAssetId("deepseek")).toBe("deepseek");
    expect(resolveProviderLogoAssetId("opencode")).toBe("opencode");
  });

  it("returns null for providers without a logo asset", () => {
    expect(resolveProviderLogoAssetId("unknown-agent")).toBeNull();
  });

  it("maps newly sourced agent logos by direct id", () => {
    expect(resolveProviderLogoAssetId("cursor")).toBe("cursor");
    expect(resolveProviderLogoAssetId("workbuddy")).toBe("workbuddy");
    expect(resolveProviderLogoAssetId("droid")).toBe("droid");
    expect(resolveProviderLogoAssetId("kiro")).toBe("kiro");
    expect(resolveProviderLogoAssetId("gemini")).toBe("gemini");
    expect(resolveProviderLogoAssetId("antigravity")).toBe("antigravity");
    expect(resolveProviderLogoAssetId("factory")).toBe("droid");
    expect(resolveProviderLogoAssetId("cline")).toBe("cline");
  });
});

describe("providerLogoFallbackInitial", () => {
  it("uses the first character of the provider id", () => {
    expect(providerLogoFallbackInitial("cursor")).toBe("C");
    expect(providerLogoFallbackInitial("")).toBe("?");
  });
});
