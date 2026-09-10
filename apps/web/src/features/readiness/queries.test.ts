import { describe, expect, it } from "vitest";
import { isReadinessOperationTerminal, manualReconcilePayload } from "./queries";

describe("manualReconcilePayload", () => {
  it("builds a manual trigger reconcile payload", () => {
    expect(manualReconcilePayload("/work")).toEqual({
      workspace: "/work",
      trigger: "manual",
    });
  });

  it("forwards a null workspace unchanged", () => {
    expect(manualReconcilePayload(null)).toEqual({
      workspace: null,
      trigger: "manual",
    });
  });
});

describe("readiness operation status", () => {
  it("recognizes terminal and polling states", () => {
    expect(isReadinessOperationTerminal({ status: "running" } as never)).toBe(false);
    expect(isReadinessOperationTerminal({ status: "completed" } as never)).toBe(true);
    expect(isReadinessOperationTerminal({ status: "failed" } as never)).toBe(true);
    expect(isReadinessOperationTerminal({ status: "superseded" } as never)).toBe(true);
  });
});
