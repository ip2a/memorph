import { useState } from "react";
import { ChevronRightIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { PageError } from "@/components/shared/page-states";
import { SectionHeading } from "@/components/shared/section-heading";
import { cn } from "@/lib/utils";
import type {
  AgentManagementEntry,
  ProviderConfigIssue,
  ProviderConfigRow,
  ProviderConfigTone,
  ProviderConfigView,
  ProviderSettingItem,
} from "@/lib/types";
import { useProviderConfigView } from "@/features/agents/queries";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "ghost";

function toneVariant(tone: ProviderConfigTone): BadgeVariant {
  switch (tone) {
    case "ok":
      return "secondary";
    case "warning":
      return "outline";
    case "danger":
      return "destructive";
    case "muted":
      return "ghost";
  }
}

function renderValue(value: unknown): string {
  if (value === null || value === undefined) return "-";
  if (typeof value === "boolean") return value ? "yes" : "no";
  return String(value);
}

function FactRow({ row }: { row: ProviderConfigRow }) {
  return (
    <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-1">
        <strong className="text-sm font-medium">{row.label}</strong>
        {row.hint ? <span className="text-muted-foreground text-xs">{row.hint}</span> : null}
      </div>
      <div className="text-muted-foreground flex min-w-0 items-center gap-2 break-words font-mono text-xs">
        {row.tone ? (
          <Badge variant={toneVariant(row.tone)}>{renderValue(row.value)}</Badge>
        ) : (
          <span>{renderValue(row.value)}</span>
        )}
      </div>
    </div>
  );
}

function IssueRow({ issue }: { issue: ProviderConfigIssue }) {
  return (
    <li className="flex items-start gap-2">
      <Badge variant={toneVariant(issue.tone)} className="mt-0.5 capitalize">
        {issue.tone}
      </Badge>
      <span className="text-muted-foreground text-xs">{issue.message}</span>
    </li>
  );
}

function ConfigViewContent({ view }: { view: ProviderConfigView }) {
  const sources = view.sources?.filter((source) => source.path).map((source) =>
    source.exists ? source.path : `${source.path} (missing)`,
  );
  return (
    <div className="flex flex-col gap-4">
      {sources && sources.length > 0 ? (
        <div className="text-muted-foreground text-xs">Read from {sources.join(", ")}</div>
      ) : null}
      {view.sections?.map((section) => (
        <div key={section.label} className="flex flex-col">
          <strong className="text-foreground pb-1 text-sm font-medium">{section.label}</strong>
          {section.rows.map((row) => (
            <FactRow key={row.label} row={row} />
          ))}
        </div>
      ))}
      {view.issues && view.issues.length > 0 ? (
        <ul className="flex flex-col gap-2">
          {view.issues.map((issue, index) => (
            <IssueRow key={index} issue={issue} />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function ConfigViewPanel({ providerId, view }: { providerId: string; view: ProviderSettingItem }) {
  const [open, setOpen] = useState(false);
  // Lazy: the content query only fires once the panel is expanded, so opening a
  // view never blocks the rest of the agent page.
  const result = useProviderConfigView(providerId, view.id, open);

  return (
    <div className="flex flex-col border-b">
      <button
        type="button"
        onClick={() => setOpen((next) => !next)}
        aria-expanded={open}
        className="flex items-center gap-2 py-3 text-left"
      >
        <ChevronRightIcon
          data-icon="inline-start"
          className={cn("transition-transform", open && "rotate-90")}
        />
        <span className="flex min-w-0 flex-col gap-0.5">
          <strong className="text-sm font-medium">{view.title}</strong>
          <span className="text-muted-foreground truncate text-xs">{view.description}</span>
        </span>
      </button>
      {open ? (
        <div className="flex flex-col gap-3 pb-3 pl-6">
          {result.isLoading ? <Skeleton className="h-16 w-full" /> : null}
          {result.error ? (
            <PageError title={`${view.title} failed to load`} message={result.error.message} />
          ) : null}
          {result.data ? <ConfigViewContent view={result.data} /> : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * Renders the `View`-kind provider settings as expandable, lazily-loaded panels.
 * Declarations ride the existing agent-detail payload (cheap metadata); each
 * panel's content is fetched on demand through a gated query, so this block never
 * blocks the page render.
 */
export function ConfigViewsBlock({ provider }: { provider: AgentManagementEntry }) {
  const views = (provider.settings || []).filter((setting) => setting.kind === "view");
  if (views.length === 0) return null;
  return (
    <section className="flex flex-col gap-4 border-t pt-5" data-config-views>
      <SectionHeading
        title="Configuration"
        description="Read-only inspection of this provider's own config — MCP servers, plugins, and more."
        className="border-b-0 pb-0"
      />
      {views.map((view) => (
        <ConfigViewPanel key={view.id} providerId={provider.provider_id} view={view} />
      ))}
    </section>
  );
}
