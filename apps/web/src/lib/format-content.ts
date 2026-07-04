function escapeHtml(text: string) {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function looksLikeJson(text: string) {
  const trimmed = String(text || "").trim();
  if (!trimmed) return false;
  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
    try {
      JSON.parse(trimmed);
      return true;
    } catch {
      return false;
    }
  }
  return false;
}

function highlightJson(jsonText: string) {
  const pretty = JSON.stringify(JSON.parse(jsonText), null, 2);
  let out = "";
  let index = 0;

  while (index < pretty.length) {
    const char = pretty[index];
    if (char === '"') {
      let end = index + 1;
      while (end < pretty.length) {
        if (pretty[end] === "\\" && end + 1 < pretty.length) {
          end += 2;
          continue;
        }
        if (pretty[end] === '"') {
          end += 1;
          break;
        }
        end += 1;
      }
      const token = pretty.slice(index, end);
      const isKey = pretty.slice(end).match(/^\s*:/);
      const escaped = escapeHtml(token);
      out += isKey
        ? `<span class="json-key">${escaped}</span>`
        : `<span class="json-string">${escaped}</span>`;
      index = end;
      if (isKey) {
        const colonMatch = pretty.slice(end).match(/^\s*:/);
        if (colonMatch) {
          out += `<span class="json-colon">${escapeHtml(colonMatch[0])}</span>`;
          index += colonMatch[0].length;
        }
      }
      continue;
    }
    if (/[-\d]/.test(char)) {
      const match = pretty.slice(index).match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/);
      if (match) {
        out += `<span class="json-number">${escapeHtml(match[0])}</span>`;
        index += match[0].length;
        continue;
      }
    }
    if (/[a-z]/.test(char)) {
      const match = pretty.slice(index).match(/^(true|false|null)/);
      if (match) {
        out += `<span class="json-literal">${match[0]}</span>`;
        index += match[0].length;
        continue;
      }
    }
    out += escapeHtml(char);
    index += 1;
  }

  return out;
}

export function renderHighlightedJson(value: unknown) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  if (!looksLikeJson(text)) return null;
  try {
    return highlightJson(text);
  } catch {
    return null;
  }
}
