import { useMemo } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { SessionGroup } from "@/lib/types";
import { cn } from "@/lib/utils";

const GROUP_SHADES = [
  "bg-foreground/18",
  "bg-foreground/28",
  "bg-foreground/38",
  "bg-foreground/48",
  "bg-foreground/58",
  "bg-foreground/68",
  "bg-muted-foreground/45",
  "bg-muted-foreground/60",
] as const;

export function scrollToHomeSessionGroup(providerId: string) {
  const item = document.querySelector(`[data-home-session-group="${providerId}"]`);
  if (!item) return false;
  item.scrollIntoView({ behavior: "smooth", block: "start" });
  return true;
}

function groupLabel(group: SessionGroup) {
  return group.provider_name || group.provider_id;
}

function groupShade(providerId: string) {
  let hash = 0;
  for (let index = 0; index < providerId.length; index += 1) {
    hash = providerId.charCodeAt(index) + ((hash << 5) - hash);
  }
  return GROUP_SHADES[Math.abs(hash) % GROUP_SHADES.length];
}

export function HomeSessionGroupRail({
  groups,
  className,
}: {
  groups: SessionGroup[];
  className?: string;
}) {
  const weights = useMemo(
    () => groups.map((group) => Math.max(group.sessions.length, 1)),
    [groups],
  );
  const totalWeight = weights.reduce((sum, weight) => sum + weight, 0) || 1;

  if (groups.length < 2) return null;

  return (
    <nav
      className={cn("flex h-full min-h-0 w-4 shrink-0 flex-col gap-px py-3 pl-1.5 pr-1", className)}
      aria-label="Jump to agent groups"
      data-home-session-group-rail
    >
      {groups.map((group, index) => {
        const label = groupLabel(group);
        const flexGrow = Math.max(0.08, weights[index] / totalWeight);

        return (
          <Tooltip key={group.provider_id}>
            <TooltipTrigger asChild>
              <button
                type="button"
                className={cn(
                  "min-h-1 w-full shrink-0 rounded-[2px] border-0 p-0 opacity-90 transition-opacity hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring/50",
                  groupShade(group.provider_id),
                )}
                style={{
                  flex: `${flexGrow} 1 0`,
                }}
                aria-label={`Jump to ${label}`}
                data-home-session-group-target={group.provider_id}
                onClick={() => scrollToHomeSessionGroup(group.provider_id)}
              />
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={6} className="font-mono">
              {label}
              <span className="text-background/70"> · {group.sessions.length}</span>
            </TooltipContent>
          </Tooltip>
        );
      })}
    </nav>
  );
}
