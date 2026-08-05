import { clampSkillsCatalogPageSize } from "@/features/skills/skills-catalog-page-size";
import type { SettingsPayload, UpdateSettingsPayload } from "@/lib/types";

function clampSessionsPerProvider(value: number) {
  return Math.max(1, Math.min(200, Number(value || 12)));
}

function clampPort(port: number, fallback: number) {
  return Math.max(1, Math.min(65535, Number(port || fallback)));
}

export function buildUpdateSettingsPayloadFromMeta(
  settings: SettingsPayload,
  patch: Partial<UpdateSettingsPayload> = {},
): UpdateSettingsPayload {
  return {
    sessions_per_provider: clampSessionsPerProvider(
      patch.sessions_per_provider ?? settings.sessions_per_provider,
    ),
    skills_catalog_page_size: clampSkillsCatalogPageSize(
      patch.skills_catalog_page_size ?? settings.skills_catalog_page_size,
    ),
    language: patch.language ?? settings.language ?? "auto",
    show_opencode_subagents:
      patch.show_opencode_subagents ??
      settings.show_opencode_subagents ??
      false,
    sort_providers_by_session_count:
      patch.sort_providers_by_session_count ??
      settings.sort_providers_by_session_count ??
      false,
    default_backup_dir:
      patch.default_backup_dir ?? settings.default_backup_dir ?? "./backups",
    logging: patch.logging ?? {
      max_size_bytes: Number(settings.logging?.max_size_bytes ?? 5 * 1024 * 1024),
      retention_days:
        settings.logging?.retention_days == null
          ? null
          : Number(settings.logging.retention_days),
    },
    home_buttons: patch.home_buttons ?? {
      view: settings.home_buttons?.view !== false,
      compress: settings.home_buttons?.compress !== false,
      switch: settings.home_buttons?.switch !== false,
      export: settings.home_buttons?.export !== false,
      sync: settings.home_buttons?.sync !== false,
      delete: settings.home_buttons?.delete !== false,
    },
    home_session_layout:
      patch.home_session_layout ?? settings.home_session_layout ?? "tabs",
    agent_order: patch.agent_order ?? settings.agent_order ?? [],
    primary_agents: patch.primary_agents ?? settings.primary_agents ?? [],
    server: patch.server ?? {
      web_port: clampPort(settings.server?.web_port ?? 3737, 3737),
      api_port: clampPort(settings.server?.api_port ?? 3223, 3223),
    },
  };
}
