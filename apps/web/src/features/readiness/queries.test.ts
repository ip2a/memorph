import { describe, expect, it } from "vitest";
import { isReadinessOperationTerminal, manualReconcilePayload, shouldStartStartupReconcile } from "./queries";

describe("readiness startup policy", () => {
  it("does not reconcile ready workspaces or an existing operation", () => {
    expect(shouldStartStartupReconcile("ready", null, "/work", null)).toBe(false);
    expect(shouldStartStartupReconcile("partial", "op-1", "/work", null)).toBe(false);
  });

  it("reconciles incomplete data only once per workspace", () => {
    expect(shouldStartStartupReconcile("partial", null, "/work", null)).toBe(true);
    expect(shouldStartStartupReconcile("partial", null, "/work", "/work")).toBe(false);
    expect(shouldStartStartupReconcile("degraded", null, null, null)).toBe(true);
  });

  it("uses a foreground manual overview reconcile", () => {
    expect(manualReconcilePayload("/work")).toEqual({
      workspace: "/work",
      focus: "overview",
      priority: "foreground",
      trigger: "manual",
    });
  });
});

describe("readiness operation status", () => {
  it("recognizes terminal and polling states", () => {
    expect(isReadinessOperationTerminal({ status: "running" } as never)).toBe(false);
    expect(isReadinessOperationTerminal({ status: "completed" } as never)).toBe(true);
    expect(isReadinessOperationTerminal({ status: "failed" } as never)).toBe(true);
  });
});
