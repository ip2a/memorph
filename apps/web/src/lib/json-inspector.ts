export type JsonField = {
  key: string;
  value: unknown;
  type: "array" | "null" | "object" | "boolean" | "number" | "string" | "undefined";
  depth: number;
};

function jsonValueType(value: unknown): JsonField["type"] {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as JsonField["type"];
}

export function normalizeJsonDocument(value: unknown):
  | { ok: true; data: unknown }
  | { ok: false; error: string } {
  if (typeof value !== "string") return { ok: true, data: value };

  try {
    return { ok: true, data: JSON.parse(value) };
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid JSON",
    };
  }
}

export function buildJsonFieldList(value: unknown): JsonField[] {
  const fields: JsonField[] = [];

  function visit(entry: unknown, depth: number) {
    if (entry === null || typeof entry !== "object") return;

    for (const [key, child] of Object.entries(entry)) {
      fields.push({ key, value: child, type: jsonValueType(child), depth });
      visit(child, depth + 1);
    }
  }

  visit(value, 0);
  return fields;
}

export function formatJsonDocument(value: unknown) {
  return JSON.stringify(value, null, 2);
}
