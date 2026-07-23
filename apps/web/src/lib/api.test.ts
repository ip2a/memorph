import { afterEach, describe, expect, it, vi } from "vitest";
import { api, ApiError, isBackendUnavailableError } from "./api";

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

describe("isBackendUnavailableError", () => {
  it("detects gateway and network failures", () => {
    expect(isBackendUnavailableError(new ApiError("HTTP 502", 502))).toBe(true);
    expect(isBackendUnavailableError(new ApiError("HTTP 503", 503))).toBe(true);
    expect(isBackendUnavailableError(new ApiError("HTTP 504", 504))).toBe(true);
    expect(isBackendUnavailableError(new TypeError("Failed to fetch"))).toBe(true);
    expect(isBackendUnavailableError(new Error("Load failed"))).toBe(true);
  });

  it("ignores ordinary API failures", () => {
    expect(isBackendUnavailableError(new ApiError("bad request", 400))).toBe(false);
    expect(isBackendUnavailableError(new ApiError("HTTP 500", 500))).toBe(false);
    expect(isBackendUnavailableError(new Error("Settings not loaded"))).toBe(false);
  });
});
