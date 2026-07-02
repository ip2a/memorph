import {
  BotIcon,
  BoxesIcon,
  DatabaseZapIcon,
  GitBranchIcon,
  HomeIcon,
  LayersIcon,
  Settings2Icon,
} from "lucide-react";

export type AppRoute = {
  title: string;
  description: string;
  href: string;
  icon: React.ComponentType;
  phase: string;
};

export const appRoutes: AppRoute[] = [
  {
    title: "Home",
    description: "Provider groups, recent sessions, and workspace entry.",
    href: "/",
    icon: HomeIcon,
    phase: "Phase 4",
  },
  {
    title: "Sessions",
    description: "Session details, event rendering, and provider-scoped views.",
    href: "/sessions",
    icon: LayersIcon,
    phase: "Phase 4",
  },
  {
    title: "Sync",
    description: "Sync group list, group detail, and bind workflows.",
    href: "/sync",
    icon: GitBranchIcon,
    phase: "Phase 4",
  },
  {
    title: "Manager",
    description: "Session and workspace management with filters and bulk actions.",
    href: "/manager",
    icon: BoxesIcon,
    phase: "Phase 4",
  },
  {
    title: "Compression",
    description: "Compression overview, archive detail, run and restore workflows.",
    href: "/compression",
    icon: DatabaseZapIcon,
    phase: "Phase 4",
  },
  {
    title: "Agents",
    description: "Agent management, provider detail, and settings controls.",
    href: "/agents",
    icon: BotIcon,
    phase: "Phase 4",
  },
  {
    title: "Hooks",
    description: "Hook providers, diagnostics, and operational controls.",
    href: "/hooks",
    icon: Settings2Icon,
    phase: "Phase 4",
  },
];

export function routeTitle(pathname: string) {
  const exact = appRoutes.find((route) => route.href === pathname);
  if (exact) return exact.title;
  if (pathname.startsWith("/sessions/")) return "Session Detail";
  if (pathname.startsWith("/sync/")) return "Sync Detail";
  return "Home";
}
