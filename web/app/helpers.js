export function createFormatHelpers(getLanguage) {
  function shortId(value) {
    const text = String(value || "");
    if (text.length <= 12) return text;
    return `${text.slice(0, 8)}...`;
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function escapeAttr(value) {
    return escapeHtml(String(value ?? "")).replaceAll("'", "&#39;");
  }

  function markdown(text) {
    const lines = String(text || "").split("\n");
    const chunks = [];
    let inCode = false;
    let codeLines = [];

    for (const line of lines) {
      if (line.startsWith("```")) {
        if (inCode) {
          chunks.push(`<pre class="code-block">${escapeHtml(codeLines.join("\n"))}</pre>`);
          codeLines = [];
          inCode = false;
        } else {
          inCode = true;
        }
        continue;
      }

      if (inCode) {
        codeLines.push(line);
        continue;
      }

      if (!line.trim()) {
        chunks.push("<p><br></p>");
        continue;
      }

      chunks.push(`<p>${escapeHtml(line).replace(/`([^`]+)`/g, '<span class="inline-code">$1</span>')}</p>`);
    }

    if (inCode) {
      chunks.push(`<pre class="code-block">${escapeHtml(codeLines.join("\n"))}</pre>`);
    }

    return chunks.join("");
  }

  function formatDate(value) {
    if (!value) return "—";
    const date = typeof value === "number" ? new Date(value) : new Date(value);
    if (Number.isNaN(date.getTime())) return String(value);
    return date.toLocaleString(getLanguage() === "zh" ? "zh-CN" : "en-US");
  }

  function formatValue(value) {
    if (typeof value === "string" && /^\d{4}-\d{2}-\d{2}T/.test(value)) return formatDate(value);
    return String(value);
  }

  function formatBytes(value) {
    if (value === null || value === undefined || value === "") return "—";
    const units = ["B", "KB", "MB", "GB"];
    let size = Number(value);
    if (!Number.isFinite(size)) return "—";
    let index = 0;
    while (size >= 1024 && index < units.length - 1) {
      size /= 1024;
      index += 1;
    }
    return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
  }

  function formatRatio(value) {
    const number = Number(value);
    if (!Number.isFinite(number)) return "—";
    return `${(number * 100).toFixed(1)}%`;
  }

  function workspaceName(path) {
    if (!path) return "";
    return path.replace(/[\\/]$/, "").split(/[\\/]/).pop() || path;
  }

  function emptyToNull(value) {
    const text = String(value || "").trim();
    return text ? text : null;
  }

  function numberOrNull(value) {
    const text = String(value || "").trim();
    if (!text) return null;
    const number = Number(text);
    return Number.isFinite(number) ? number : null;
  }

  return {
    shortId,
    markdown,
    formatDate,
    formatValue,
    formatBytes,
    formatRatio,
    workspaceName,
    emptyToNull,
    numberOrNull,
    escapeHtml,
    escapeAttr,
  };
}
