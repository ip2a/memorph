export const migrationPages = {
  sessions: {
    title: "Sessions",
    description: "Session list, detail shell, event rendering, and provider scoped navigation.",
    legacySource: "归档/web-legacy/app/session_sync.js",
    workflows: ["Session list", "Session detail", "Events", "Rename", "Delete"],
  },
  sync: {
    title: "Sync",
    description: "Sync group list, group detail, bind and unbind workflows.",
    legacySource: "归档/web-legacy/app/session_sync.js",
    workflows: ["Sync list", "Sync detail", "Bind", "Unbind", "Rename"],
  },
  manager: {
    title: "Manager",
    description: "Session and workspace management with filters, selection, and bulk actions.",
    legacySource: "归档/web-legacy/app/manager_compression.js",
    workflows: ["Sessions", "Workspaces", "Filters", "Bulk selection", "Clean", "Backup"],
  },
  compression: {
    title: "Compression",
    description: "Compression overview, archive details, run, restore, and cleanup flows.",
    legacySource: "归档/web-legacy/app/manager_compression.js",
    workflows: ["Overview", "Archive detail", "Run", "Restore"],
  },
  agents: {
    title: "Agents",
    description: "Agent providers, management controls, and settings entry points.",
    legacySource: "归档/web-legacy/app/agents_settings.js",
    workflows: ["Providers", "Agents", "Settings", "Provider controls"],
  },
};
