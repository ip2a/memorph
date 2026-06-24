export async function api(path, options = {}) {
  const request = {
    method: options.method || "GET",
    headers: {
      Accept: "application/json",
    },
  };
  if (options.body !== undefined) {
    request.headers["Content-Type"] = "application/json";
    request.body = JSON.stringify(options.body);
  }

  const response = await fetch(path, request);
  const raw = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(raw?.error || `HTTP ${response.status}`);
  }
  if (raw?.ok) return raw.data;
  return raw;
}
