import { describe, expect, it } from "vitest";
import type { ReadinessOperation, ReadinessPayload } from "@/lib/types";

describe("readiness backend fixtures", () => {
  it("accepts the serialized readiness response shape", () => {
    const serialized = `{"workspace":null,"state":"partial","active_operation_id":"op-123","recommended_focus":"sessions","reconcile_required":"incremental","reconcile_reason":"stale_signatures","last_full_at":1700000000000,"last_incremental_at":1700000100000,"phases":{"foundation":{"state":"ready","message":"SQLite is available"},"agents":{"state":"ready","message":null},"sessions":{"state":"partial","message":"Session history is still indexing"},"session_stats":{"state":"partial","message":"Session statistics are still being completed"},"skills":{"state":"ready"},"usage":{"state":"degraded","message":"Usage data may be incomplete"},"derived":{"state":"partial","message":null}}}`;
    const response = JSON.parse(serialized) as ReadinessPayload;

    expect(Object.keys(response.phases).sort()).toEqual([
      "agents",
      "derived",
      "foundation",
      "session_stats",
      "sessions",
      "skills",
      "usage",
    ]);
    expect(response.phases.sessions).toEqual({
      state: "partial",
      message: "Session history is still indexing",
    });
    expect(response.phases.agents.message).toBeNull();
    expect(response.reconcile_required).toBe("incremental");
    expect(response.reconcile_reason).toBe("stale_signatures");
  });

  it("accepts the serialized operation response shape", () => {
    const serialized = `{"operation_id":"op-123","status":"failed","readiness":{"workspace":"/work/demo","state":"degraded","active_operation_id":null,"recommended_focus":"skills","phases":{"foundation":{"state":"ready"},"agents":{"state":"ready"},"sessions":{"state":"ready"},"session_stats":{"state":"ready"},"skills":{"state":"error","message":"Skill directory unavailable"},"usage":{"state":"ready"},"derived":{"state":"partial","message":"Waiting for skills"}}},"trigger":"startup","plan":"incremental","current_phase":"skills","completed_phases":["foundation","agents","sessions","session_stats"],"running_phases":[],"pending_phases":["usage","derived"],"failures":[{"phase":"skills","message":"Skill directory unavailable"}]}`;
    const operation = JSON.parse(serialized) as ReadinessOperation;

    expect(operation).toMatchObject({
      trigger: "startup",
      plan: "incremental",
      current_phase: "skills",
      completed_phases: ["foundation", "agents", "sessions", "session_stats"],
      failures: [{ phase: "skills", message: "Skill directory unavailable" }],
    });
    expect(operation).not.toHaveProperty("priority");
    expect(operation.failures[0]).not.toHaveProperty("reason");
  });
});
