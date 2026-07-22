import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, api } from "./api";

afterEach(() => vi.restoreAllMocks());

describe("api", () => {
  it("unwraps successful envelopes", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ ok: true, data: { value: 1 } }), { status: 200 }),
    ));

    await expect(api<{ value: number }>("/api/test")).resolves.toEqual({ value: 1 });
  });

  it("rejects failed envelopes even on HTTP 200", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ ok: false, error: "bad request" }), { status: 200 }),
    ));

    await expect(api("/api/test")).rejects.toMatchObject({
      message: "bad request",
      status: 200,
    });
  });

  it("uses the HTTP status when a failed response is not JSON", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response("not json", { status: 500 }),
    ));

    await expect(api("/api/test")).rejects.toMatchObject({
      message: "HTTP 500",
      status: 500,
    });
  });
});
