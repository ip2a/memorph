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
    let inList = false;
    let listItems = [];

    function flushList() {
      if (!listItems.length) return;
      chunks.push(`<ul>${listItems.join("")}</ul>`);
      listItems = [];
      inList = false;
    }

    function inlineMarkup(line) {
      return escapeHtml(line)
        .replace(/`([^`]+)`/g, '<code class="inline-code">$1</code>')
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/(^|[^*])\*([^*]+)\*(?![*])/g, '$1<em>$2</em>')
        .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
    }

    for (const line of lines) {
      if (line.startsWith("```")) {
        flushList();
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

      const headingMatch = line.match(/^(#{1,6})\s+(.*)$/);
      if (headingMatch) {
        flushList();
        const level = headingMatch[1].length;
        chunks.push(`<h${level}>${escapeHtml(headingMatch[2])}</h${level}>`);
        continue;
      }

      const listMatch = line.match(/^(\s*)[-*+]\s+(.*)$/);
      if (listMatch) {
        inList = true;
        listItems.push(`<li>${inlineMarkup(listMatch[2])}</li>`);
        continue;
      }

      flushList();

      if (!line.trim()) {
        chunks.push("<p><br></p>");
        continue;
      }

      chunks.push(`<p>${inlineMarkup(line)}</p>`);
    }

    flushList();

    if (inCode) {
      chunks.push(`<pre class="code-block">${escapeHtml(codeLines.join("\n"))}</pre>`);
    }

    return chunks.join("");
  }

  function looksLikeJson(text) {
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

  function looksLikeMarkdown(text) {
    const sample = String(text || "").trim();
    if (!sample) return false;
    const markdownPatterns = [
      /^#{1,6}\s+/m,
      /^[-*+]\s+/m,
      /^```/m,
      /\*\*[^*]+\*\*/,
      /\[([^\]]+)\]\(([^)]+)\)/,
      /^>\s+/m,
      /^\d+\.\s+/m,
    ];
    return markdownPatterns.some((pattern) => pattern.test(sample));
  }

  function highlightJson(jsonText) {
    const pretty = JSON.stringify(JSON.parse(jsonText), null, 2);
    let out = "";
    let i = 0;
    while (i < pretty.length) {
      const char = pretty[i];
      if (char === '"') {
        let j = i + 1;
        while (j < pretty.length) {
          if (pretty[j] === '\\' && j + 1 < pretty.length) {
            j += 2;
            continue;
          }
          if (pretty[j] === '"') {
            j += 1;
            break;
          }
          j += 1;
        }
        const token = pretty.slice(i, j);
        const isKey = pretty.slice(j).match(/^\s*:/);
        const escaped = escapeHtml(token);
        out += isKey
          ? `<span class="json-key">${escaped}</span><span class="json-colon">:</span>`
          : `<span class="json-string">${escaped}</span>`;
        i = j;
        if (isKey) {
          const colonMatch = pretty.slice(j).match(/^\s*:/);
          if (colonMatch) {
            i += colonMatch[0].indexOf(':') + 1;
          }
        }
        continue;
      }
      if (/[-\d]/.test(char)) {
        const match = pretty.slice(i).match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/);
        if (match) {
          out += `<span class="json-number">${escapeHtml(match[0])}</span>`;
          i += match[0].length;
          continue;
        }
      }
      if (/[a-z]/.test(char)) {
        const match = pretty.slice(i).match(/^(true|false|null)/);
        if (match) {
          out += `<span class="json-literal">${match[0]}</span>`;
          i += match[0].length;
          continue;
        }
      }
      out += escapeHtml(char);
      i += 1;
    }
    return out;
  }

  function formatJson(text) {
    try {
      return `<pre class="code-block json-block"><code>${highlightJson(text)}</code></pre>`;
    } catch {
      return `<pre class="code-block"><code>${escapeHtml(text)}</code></pre>`;
    }
  }

  function formatContent(text) {
    const raw = String(text || "");
    if (looksLikeJson(raw)) {
      return { kind: "json", html: formatJson(raw), raw };
    }
    if (looksLikeMarkdown(raw)) {
      return { kind: "markdown", html: markdown(raw), raw };
    }
    return { kind: "text", html: escapeHtml(raw), raw };
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
    formatContent,
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
