export function createSessionSyncModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  markdown,
  formatContent,
  renderAgentDetailRow,
  renderMetaLine,
  findSyncRef,
}) {
  function formatToolInputHint(input) {
    if (!input || typeof input !== "object") return "";
    const command = input.command || input.file_path || input.path;
    return command ? ` — ${String(command).slice(0, 120)}` : "";
  }

  function getBlockLabel(block) {
    switch (block.type) {
      case "text": return "";
      case "thinking": return t("thinking");
      case "tool_call": return `${t("toolUse")}: ${block.name || ""}`.replace(/:\s$/, "");
      case "tool_result": return t("toolResult");
      case "patch": return "Patch";
      case "command": return "Command";
      case "command_result": return "Command Result";
      case "file": return t("file");
      case "image": return t("image");
      case "provider_payload": return block.kind || "payload";
      case "unknown": return t("details");
      default: return "";
    }
  }

  function getBlockLabels(blocks) {
    return (blocks || []).map(getBlockLabel).filter(Boolean);
  }

  function countLines(text) {
    if (!text) return 0;
    return text.split("\n").length;
  }

  function renderFormattedBlock(messageIndex, blockIndex, kind, clamp, formatted, raw) {
    return `
      <div class="content-block content-${kind} ${clamp}" data-block-kind="${kind}" data-message-index="${messageIndex}" data-block-index="${blockIndex}">
        <div class="content-formatted">${formatted}</div>
        <div class="content-raw" hidden><pre><code>${escapeHtml(raw)}</code></pre></div>
      </div>`;
  }

  function renderDetailBlock(block, messageIndex, blockIndex) {
    const clampIf = (text, limit = 3) => {
      const lines = countLines(text || "");
      return lines > limit ? "is-clamped" : "";
    };

    switch (block.type) {
      case "text": {
        const clamp = clampIf(block.text, 3);
        return `<div class="content-block content-text ${clamp}">${markdown(block.text || "")}</div>`;
      }
      case "thinking": {
        const clamp = clampIf(block.text, 3);
        return `<div class="content-block content-thinking ${clamp}"><p>${escapeHtml(block.text || "")}</p></div>`;
      }
      case "tool_call": {
        const text = JSON.stringify(
          { tool_call_id: block.tool_call_id, name: block.name, input: block.input },
          null,
          2
        );
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "tool", clamp, formatted.html, text);
      }
      case "tool_result": {
        const text = block.content || "";
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "tool", clamp, formatted.html, text);
      }
      case "patch": {
        const text = block.diff_text || JSON.stringify(
          { summary: block.summary, files: block.files, hash: block.hash },
          null,
          2
        );
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "patch", clamp, formatted.html, text);
      }
      case "command": {
        const text = JSON.stringify(
          { command: block.command, argv: block.argv, cwd: block.cwd },
          null,
          2
        );
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "command", clamp, formatted.html, text);
      }
      case "command_result": {
        const text = JSON.stringify(
          { command: block.command, exit_code: block.exit_code, stdout: block.stdout, stderr: block.stderr },
          null,
          2
        );
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "command-result", clamp, formatted.html, text);
      }
      case "file": {
        const clamp = block.content ? clampIf(block.content) : "";
        const raw = block.content || "";
        const formatted = raw ? formatContent(raw) : { html: "", raw: "" };
        return renderFormattedBlock(
          messageIndex,
          blockIndex,
          "file",
          clamp,
          `<code>${escapeHtml(block.path || "")}</code>${formatted.html ? `<div class="file-content">${formatted.html}</div>` : ""}`,
          `${block.path || ""}${block.path && raw ? "\n" : ""}${raw}`
        );
      }
      case "image":
        return `<div class="content-block content-image"><code>${escapeHtml(block.path || block.mime_type || "")}</code></div>`;
      case "provider_payload": {
        const text = JSON.stringify(block.payload ?? {}, null, 2);
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "provider-payload", clamp, formatted.html, text);
      }
      case "unknown": {
        const text = JSON.stringify(block.raw ?? block, null, 2);
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "unknown", clamp, formatted.html, text);
      }
      default: {
        const text = JSON.stringify(block, null, 2);
        const formatted = formatContent(text);
        const clamp = clampIf(text);
        return renderFormattedBlock(messageIndex, blockIndex, "unknown", clamp, formatted.html, text);
      }
    }
  }

  function findEventStats(eventId, eventIndex) {
    const events = state.session?.stats?.events;
    if (!events) return null;
    return events.find((item) => item.event_id === eventId)
      || events.find((item, idx) => idx === eventIndex && !eventId)
      || null;
  }

  function renderEventStats(stats) {
    if (!stats) return "";
    const visibleChars = t("statsVisibleChars", { count: stats.visible_char_count });
    const visibleBytes = t("statsVisibleBytes", { size: formatBytes(stats.visible_byte_size) });
    const totalChars = t("statsTotalChars", { count: stats.char_count });
    const totalBytes = t("statsTotalBytes", { size: formatBytes(stats.byte_size) });
    const title = t("statsTooltip", { visibleChars, visibleBytes, totalChars, totalBytes });
    const label = t("statsLabel", { size: formatBytes(stats.byte_size), count: stats.char_count });
    return ` · <span class="msg-stats" title="${escapeAttr(title)}">${label}</span>`;
  }

  function renderDetailEvent(event, index) {
    const blocks = (event.blocks || []).map((block, blockIndex) => renderDetailBlock(block, index, blockIndex)).join("");
    const role = (event.role || "unknown").replaceAll("_", " ");
    const kind = (event.kind || "unknown").replaceAll("_", " ");
    const blockLabels = getBlockLabels(event.blocks);
    const labelPart = blockLabels.length
      ? ` · ${blockLabels.map((label) => `<span class="msg-block-label">${escapeHtml(label)}</span>`).join(" · ")}`
      : "";
    const stats = findEventStats(event.id, index);
    const statsPart = renderEventStats(stats);
    return `
      <article class="msg-item" data-message-index="${index}" data-role="${escapeAttr(event.role || "unknown")}">
        <header class="msg-header">
          <span class="msg-header-main">
            <span class="msg-role">${escapeHtml(role)}</span>
            <span>${escapeHtml(kind)}</span>${labelPart}${statsPart}
          </span>
          <span class="msg-header-meta">
            <a href="#" class="text-action" data-action="copy-detail-message" data-message-index="${index}">${t("copy")}</a>
            <a href="#" class="text-action" data-action="delete-detail-message" data-message-index="${index}">${t("remove")}</a>
            <a href="#" class="text-action" data-action="toggle-detail-message" data-message-index="${index}">${t("expand")}</a>
            <a href="#" class="text-action" data-action="toggle-message-raw" data-message-index="${index}">${t("viewRaw")}</a>
            <span>${escapeHtml(formatDate(event.timestamp))}</span>
          </span>
        </header>
        <div class="msg-body">${blocks || `<p class="muted">${t("noDetails")}</p>`}</div>
      </article>`;
  }

  function estimateEventSize(event) {
    if (event.metadata?.size_bytes != null) return Number(event.metadata.size_bytes) || 0;
    return (event.blocks || []).reduce((sum, block) => {
      if (block == null) return sum;
      if (block.text != null) return sum + String(block.text).length;
      if (block.content != null) return sum + String(block.content).length;
      if (block.diff_text != null) return sum + String(block.diff_text).length;
      if (block.payload != null) return sum + JSON.stringify(block.payload).length;
      if (block.raw != null) return sum + JSON.stringify(block.raw).length;
      return sum + JSON.stringify(block).length;
    }, 0);
  }

  function renderDetailTimeline(events) {
    const sizes = (events || []).map(estimateEventSize);
    const total = sizes.reduce((sum, size) => sum + size, 0) || 1;
    const max = Math.max(1, ...sizes);
    return `
      <aside class="detail-timeline" aria-label="${escapeAttr(t("timeline") || "Timeline")}">
        ${sizes.map((size, index) => {
          const ratio = size / total;
          const intensity = max ? size / max : 0;
          const depth = Math.max(0.08, Math.min(0.85, 0.12 + intensity * 0.7));
          return `<button type="button" class="timeline-segment" data-action="scroll-to-message" data-message-index="${index}" title="${escapeAttr(formatBytes(size))}" style="--timeline-flex: ${Math.max(0.02, ratio)}; --timeline-depth: ${depth.toFixed(3)};"></button>`;
        }).join("")}
      </aside>`;
  }

  function renderRuntimeSessionRow(session) {
    const tool = session.current_tool;
    const permission = session.pending_permission;
    const question = session.pending_question;
    const detail = question?.prompt || permission?.prompt || tool?.name || session.cwd || session.provider_session_id || session.runtime_id?.[0] || "—";
    return renderAgentDetailRow(
      `${session.provider || "hook"} · ${session.status || "unknown"}`,
      tool ? `${tool.name}${formatToolInputHint(tool.input)}` : detail,
      `last event ${session.last_event_at ? formatDate(session.last_event_at) : "—"} · updated ${
        session.updated_at ? formatDate(session.updated_at) : "—"
      }`
    );
  }

  function renderHookRuntimeBadge(summary) {
    if (!summary) return "";
    const stateLabel = summary.has_pending_permission
      ? "perm"
      : summary.has_pending_question
        ? "question"
        : summary.current_tool_name || summary.status || "hook";
    const linked = Number(summary.linked_sessions || 0);
    const waiting = Number(summary.waiting_sessions || 0);
    const title = [
      `hook=${stateLabel}`,
      linked > 1 ? `linked=${linked}` : "",
      waiting > 0 ? `waiting=${waiting}` : "",
      summary.matched_by ? `matched_by=${summary.matched_by}` : "",
      summary.confidence ? `confidence=${summary.confidence}` : "",
      summary.last_event_at ? `last=${formatDate(summary.last_event_at)}` : "",
    ]
      .filter(Boolean)
      .join(" · ");
    const prefix = linked > 1 ? `${linked} ` : "";
    return `<span class="status-pill" title="${escapeAttr(title)}">${escapeHtml(`${prefix}${stateLabel}`)}</span>`;
  }

  function renderHookDiagnosisActions(provider, diagnosis) {
    const actions = diagnosis?.actions || [];
    if (!actions.length) return "";
    return `
      <div class="pill-row">
        ${actions.map((action) => {
          const key = `${provider}:${action.setting_id}`;
          const pending = !!state.agents.pendingSettings[key];
          return `
            <button
              type="button"
              class="button button-small"
              title="${escapeAttr(action.reason || "")}"
              data-action="run-agent-setting"
              data-provider="${escapeAttr(provider)}"
              data-setting-id="${escapeAttr(action.setting_id)}"
              ${pending ? "disabled" : ""}
            >${escapeHtml(pending ? t("running") : action.label)}</button>`;
        }).join("")}
      </div>`;
  }

  function renderSessionHookRuntimeBlock(provider, runtimeSessions, summary, diagnosis) {
    if (!runtimeSessions.length) {
      const diagnosisMarkup = diagnosis
        ? `
          <p>${escapeHtml(diagnosis.message || "No linked runtime session.")}</p>
          <div class="pill-row">
            <span class="pill">${escapeHtml(diagnosis.kind || "unknown")}</span>
            <span class="pill">${escapeHtml(diagnosis.provider_status || "unknown")}</span>
            ${diagnosis.provider_runtime_sessions ? `<span class="pill">${escapeHtml(`provider_rt=${diagnosis.provider_runtime_sessions}`)}</span>` : ""}
          </div>
          ${renderHookDiagnosisActions(provider, diagnosis)}`
        : `<p>Install/enable hooks to see live tool, permission, and question state for this session.</p>`;
      return `
        <section class="manager-workspace-summary">
          <div>
            <span class="eyebrow">Hook Runtime</span>
            <strong>No linked runtime session</strong>
            ${diagnosisMarkup}
          </div>
        </section>`;
    }
    const summaryMarkup = summary
      ? `<div class="pill-row">
          ${summary.matched_by ? `<span class="pill">${escapeHtml(summary.matched_by)}</span>` : ""}
          ${summary.confidence ? `<span class="pill">${escapeHtml(summary.confidence)}</span>` : ""}
        </div>`
      : "";
    const actionMarkup = renderHookDiagnosisActions(provider, diagnosis);
    return `
      <section class="manager-workspace-summary">
        <div>
          <span class="eyebrow">Hook Runtime</span>
          <strong>${escapeHtml(runtimeSessions.length === 1 ? "Linked runtime session" : `${runtimeSessions.length} linked runtime sessions`)}</strong>
          <p>Live hook state captured from the provider runtime.</p>
          ${summaryMarkup}
          ${actionMarkup}
        </div>
        <div class="settings-list agent-environment-paths">
          ${runtimeSessions.map(renderRuntimeSessionRow).join("")}
        </div>
      </section>`;
  }

  function collectCompressedArchiveRefs(view) {
    const refs = new Set((view.compressed_archive_refs || []).slice());
    (view.events || []).forEach((event) => {
      (event.blocks || []).forEach((block) => {
        if (block.type === "compressed" && block.archive_ref) {
          refs.add(block.archive_ref);
        }
      });
    });
    return refs;
  }

  function hasCompressedEvents(view) {
    return collectCompressedArchiveRefs(view).size > 0;
  }

  function renderCompressionArchiveRef(view) {
    const refs = collectCompressedArchiveRefs(view);
    if (!refs.size) return "";
    const archiveRef = [...refs][0];
    const href = `/compression?archive_ref=${encodeURIComponent(archiveRef)}`;
    return `<a class="button" href="${href}" data-nav="${href}">${t("viewCompression")}</a>`;
  }

  function renderSessionDetailView(detail, view) {
    const syncRef = findSyncRef(state.route.provider, state.route.sessionId);
    const workspace = view.workspace_dir || state.home.workspace || "";
    const sessionMeta = (state.home?.sessions || []).find(
      (session) => session.provider_id === state.route.provider && session.session_id === state.route.sessionId
    );
    const sizeBytes = sessionMeta?.size_bytes;
    const compressed = hasCompressedEvents(view);
    return `
      <section class="session-header">
        <div>
          <p class="eyebrow">${escapeHtml(providers.displayName(view.provider_id))}</p>
          <h1>${escapeHtml(view.title || view.session_id || state.route.sessionId)}</h1>
          <div class="meta-line">
            <span>id=<code data-action="copy-session-id" data-copy-text="${escapeAttr(view.session_id || state.route.sessionId)}" class="copyable-id">${escapeHtml(view.session_id || state.route.sessionId)}</code></span>
            <span>${t("messageCount")}=${view.message_count}</span>
            ${view.last_active_at ? `<span>${t("lastActiveAt")}=${escapeHtml(formatDate(view.last_active_at))}</span>` : ""}
            ${sizeBytes != null ? `<span>${t("size")}=${escapeHtml(formatBytes(sizeBytes))}</span>` : ""}
            ${workspace ? `<span>${t("workspace")}=<code>${escapeHtml(workspace)}</code></span>` : ""}
            ${syncRef ? `<a class="sync-badge" href="/sync/${syncRef}" data-nav="/sync/${syncRef}">${t("activeSync")}</a>` : ""}
          </div>
        </div>
        <div class="session-actions">
          ${compressed ? renderCompressionArchiveRef(view) : ""}
          <button type="button" data-action="compress-session" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}">${compressed ? t("compressAgain") : t("compression")}</button>
          ${syncRef ? `<a class="button" href="/sync/${syncRef}" data-nav="/sync/${syncRef}">${t("openSync")}</a>` : ""}
          <button type="button" data-action="open-sync-create" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-title="${escapeAttr(view.title || "")}">${t("syncAction")}</button>
          <button type="button" data-action="open-switch" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-workspace="${escapeAttr(workspace)}" data-title="${escapeAttr(view.title || "")}">${t("switch")}</button>
          <button type="button" data-action="open-export" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}">${t("export")}</button>
          <button type="button" data-action="open-rename" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-title="${escapeAttr(view.title || "")}">${t("rename")}</button>
          <button type="button" class="danger" data-action="open-delete" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}">${t("remove")}</button>
        </div>
      </section>
      <div class="detail-layout">
        ${renderDetailTimeline(view.events)}
        <section>
          <div class="msg-list">
            ${view.events.length ? view.events.map((event, index) => renderDetailEvent(event, index)).join("") : `<div class="empty-state">${t("noMessages")}</div>`}
          </div>
          ${
            detail.has_more_events
              ? `<div class="section-actions"><button type="button" data-action="load-more-session-events">${t("more")} (${view.events.length}/${view.event_count})</button></div>`
              : ""
          }
        </section>
      </div>`;
  }

  function renderSessionDetail() {
    if (!state.session) return `<div class="empty-state">${t("loading")}</div>`;
    const detail = state.session;
    if (!detail.view) return `<div class="empty-state">${t("loading")}</div>`;
    return renderSessionDetailView(detail, detail.view);
  }

  function renderSyncRow(group) {
    const sourceProvider = providers.get(group.source_provider);
    const href = `/sync/${encodeURIComponent(group.id)}`;
    return `
      <article class="manager-row">
        <div class="manager-row-head">
          <div class="manager-row-copy">
            <a class="manager-title-link" href="${href}" data-nav="${href}">${escapeHtml(group.title || group.id)}</a>
            <div class="manager-meta">
              <span>${escapeHtml(providers.displayName(group.source_provider) || "—")}</span>
              <span>${t("holdings")}=${group.holdings.length}</span>
              <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(group.updated_at)))}</span>
            </div>
          </div>
          <div class="row-actions">
            <a class="button" href="${href}" data-nav="${href}">${t("view")}</a>
            <button type="button" data-action="run-sync-latest" data-group-id="${escapeAttr(group.id)}">${t("syncLatest")}</button>
            <button type="button" data-action="open-sync-rename" data-group-id="${escapeAttr(group.id)}" data-title="${escapeAttr(group.title || "")}">${t("rename")}</button>
            <button type="button" class="danger" data-action="open-sync-remove" data-group-id="${escapeAttr(group.id)}">${t("remove")}</button>
          </div>
        </div>
      </article>`;
  }

  function renderSyncList() {
    const groups = state.home.syncGroups || [];
    const totalHoldings = groups.reduce((sum, group) => sum + (group.holdings?.length || 0), 0);
    return `
      <div class="manager-page-layout">
        <section class="section-panel manager-control-panel">
          <div class="manager-control-content">
            <section class="manager-workspace-summary">
              <div>
                <span class="eyebrow">${t("syncTitle")}</span>
                <strong>${t("syncGroups")}</strong>
                <p>${t("syncOverview")}</p>
              </div>
            </section>
            <section class="manager-control-bottom">
              <div class="stack">
                <div class="manager-summary-grid">
                  ${renderMetaLine(t("sessionsStat"), String(groups.length))}
                  ${renderMetaLine(t("holdings"), String(totalHoldings))}
                </div>
              </div>
            </section>
          </div>
        </section>
        <section class="section-panel manager-result-panel">
          <div class="section-heading manager-section-head">
            <div>
              <strong>${t("syncGroups")}</strong>
              <span>${groups.length}</span>
            </div>
          </div>
          <div class="manager-list">
            ${groups.length ? groups.map(renderSyncRow).join("") : `<div class="empty-state">${t("noSyncGroups")}</div>`}
          </div>
        </section>
      </div>`;
  }

  function renderHoldingCard(group, holding) {
    const provider = providers.get(holding.provider);
    const sessionHref = `/sessions/${encodeURIComponent(holding.provider)}/${encodeURIComponent(holding.session_id)}`;
    const hookRuntimeSessions = holding.hook_runtime_sessions || [];
    const hookRuntimeBadge = renderHookRuntimeBadge(holding.hook_runtime_summary);
    const hookRuntimeBlock = renderSessionHookRuntimeBlock(
      holding.provider,
      hookRuntimeSessions,
      holding.hook_runtime_summary,
      holding.hook_diagnosis
    );
    return `
      <article class="binding-card">
        <header>
          <div>
            <strong>${escapeHtml(providers.displayName(holding.provider))}</strong>
            <p class="modal-subtitle">${escapeHtml(holding.session_id)}</p>
            ${hookRuntimeBadge}
          </div>
        </header>
        <div class="stack">
          ${renderMetaLine(t("workspace"), holding.target_dir)}
          ${renderMetaLine(t("lastActiveAt"), formatDate(holding.last_active_at))}
          ${renderMetaLine(t("lastSync"), formatDate(holding.last_sync_at))}
          ${renderMetaLine(t("syncFrom"), holding.last_sync_from)}
          ${renderMetaLine(t("error"), holding.last_error)}
          ${hookRuntimeBlock}
        </div>
        <footer class="row-actions">
          <a class="button" href="${sessionHref}" data-nav="${sessionHref}">${t("openSession")}</a>
          <button type="button" data-action="open-sync-from" data-group-id="${escapeAttr(group.id)}" data-holding-id="${escapeAttr(holding.id)}" data-provider="${escapeAttr(
            providers.displayName(holding.provider)
          )}" data-session-id="${escapeAttr(holding.session_id)}">${t("syncFromThis")}</button>
          <button type="button" class="danger" data-action="open-unbind" data-group-id="${escapeAttr(group.id)}" data-holding-id="${escapeAttr(holding.id)}" data-provider="${escapeAttr(
            providers.displayName(holding.provider)
          )}" data-session-id="${escapeAttr(holding.session_id)}">${t("unbind")}</button>
        </footer>
      </article>`;
  }

  function renderSyncDetail() {
    if (!state.syncDetail) return `<div class="empty-state">${t("loading")}</div>`;
    const group = state.syncDetail;
    const sourceProvider = providers.get(group.source_provider);
    return `
      <div class="sync-layout">
        <section class="section-panel stack">
          <div class="section-heading">
            <div>
              <strong>${t("holdings")}</strong>
              <small>${group.holdings.length}</small>
            </div>
          </div>
          <div class="sync-grid">
            ${group.holdings.map((holding) => renderHoldingCard(group, holding)).join("")}
          </div>
        </section>
        <aside class="sync-detail-right">
          <article class="manager-row">
            <div class="manager-row-head">
              <div class="manager-row-copy">
                <span class="manager-title-link">${escapeHtml(group.title || group.id)}</span>
                <div class="manager-meta">
                  <span>${escapeHtml(providers.displayName(group.source_provider) || "—")}</span>
                  <span>${t("holdings")}=${group.holdings.length}</span>
                  <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(group.updated_at)))}</span>
                </div>
              </div>
            </div>
          </article>
          <section class="section-panel stack">
            <div class="section-heading">
              <div>
                <strong>${t("status")}</strong>
              </div>
            </div>
            <div class="stack">
              ${renderMetaLine(t("provider"), providers.displayName(group.source_provider))}
              ${renderMetaLine(t("syncTitle"), group.title)}
              ${renderMetaLine(t("holdings"), String(group.holdings.length))}
              ${renderMetaLine(t("createdAt"), formatDate(group.created_at))}
              ${renderMetaLine(t("updatedAt"), formatDate(group.updated_at))}
              <div class="row-actions">
                <button type="button" class="invert" data-action="run-sync-latest" data-group-id="${escapeAttr(group.id)}">${t("startExecution")}</button>
                <button type="button" data-action="open-sync-bind" data-group-id="${escapeAttr(group.id)}">${t("addHolding")}</button>
                <button type="button" data-action="open-sync-rename" data-group-id="${escapeAttr(group.id)}" data-title="${escapeAttr(group.title || "")}">${t("rename")}</button>
                <button type="button" class="danger" data-action="open-sync-remove" data-group-id="${escapeAttr(group.id)}">${t("remove")}</button>
              </div>
            </div>
          </section>
        </aside>
      </div>`;
  }

  function detailEventToText(event) {
    return (event.blocks || [])
      .map((block) => {
        if (block.type === "text") return block.text || "";
        if (block.type === "thinking") return block.text || "";
        if (block.type === "tool_result") return block.content || "";
        if (block.type === "file") return [block.path, block.content].filter(Boolean).join("\n");
        return JSON.stringify(block.raw ?? block.payload ?? block, null, 2);
      })
      .filter(Boolean)
      .join("\n\n");
  }

  return {
    detailEventToText,
    renderSessionDetail,
    renderSyncDetail,
    renderSyncList,
  };
}
