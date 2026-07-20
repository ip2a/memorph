import { useId, useMemo, useState } from "react";
import { CopyIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import { renderHighlightedJson } from "@/lib/format-content";
import { cn } from "@/lib/utils";

function formatJson(value: unknown) {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function inferJsonSchema(value: unknown): unknown {
  if (value === null) return "null";
  if (Array.isArray(value)) {
    return value.length > 0 ? [inferJsonSchema(value[0])] : ["unknown"];
  }
  if (typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, entry]) => [key, inferJsonSchema(entry)]),
    );
  }
  return typeof value;
}

function readableLabel(label: string) {
  return label
    .replaceAll("_", " ")
    .replaceAll("-", " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

function previewFooter(label: string) {
  const normalized = label.toLowerCase();
  if (normalized.includes("request")) return "Request preview";
  if (normalized.includes("response")) return "Response preview";
  if (normalized.includes("command")) return "Command preview";
  return "Payload preview";
}

async function copyJson(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success("Copied JSON");
  } catch {
    toast.error("Failed to copy JSON");
  }
}

export function SessionJsonPanel({
  value,
  label,
  className,
}: {
  value: unknown;
  label: string;
  className?: string;
}) {
  const schemaId = useId();
  const [showSchema, setShowSchema] = useState(false);
  const displayValue = useMemo(
    () => (showSchema ? inferJsonSchema(value) : value),
    [showSchema, value],
  );
  const text = formatJson(displayValue);
  const highlighted = renderHighlightedJson(displayValue);

  return (
    <Card
      className={cn(
        "flex h-full min-h-0 w-full min-w-0 flex-col gap-0 overflow-hidden rounded-xl border border-border bg-card py-0 shadow-none ring-0",
        className,
      )}
      size="sm"
    >
      <div className="flex shrink-0 items-center justify-between gap-3 border-b px-4 py-3">
        <h3 className="text-sm font-semibold tracking-tight">{readableLabel(label)}</h3>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <Switch
              id={schemaId}
              checked={showSchema}
              onCheckedChange={setShowSchema}
              size="sm"
            />
            <Label htmlFor={schemaId} className="text-xs font-normal text-muted-foreground">
              Show Schema
            </Label>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 px-2 text-xs text-muted-foreground"
            onClick={() => copyJson(text)}
          >
            <CopyIcon data-icon="inline-start" />
            Copy
          </Button>
        </div>
      </div>

      <ScrollArea className="min-h-0 min-w-0 flex-1">
        <div className="p-4">
          {highlighted ? (
            <pre
              className="json-block m-0 whitespace-pre-wrap break-words font-mono text-[13px] leading-6 [overflow-wrap:anywhere]"
              dangerouslySetInnerHTML={{ __html: `<code>${highlighted}</code>` }}
            />
          ) : (
            <pre className="m-0 whitespace-pre-wrap break-words font-mono text-[13px] leading-6 text-foreground [overflow-wrap:anywhere]">
              {text || "-"}
            </pre>
          )}
        </div>
      </ScrollArea>

      <div className="shrink-0 border-t px-4 py-2.5 text-xs text-muted-foreground">
        {previewFooter(label)}
      </div>
    </Card>
  );
}

export function SessionEventJsonColumn({
  payloads,
  className,
}: {
  payloads: Array<{ json: unknown; jsonLabel: string }>;
  className?: string;
}) {
  if (payloads.length === 1) {
    return (
      <SessionJsonPanel
        className={cn("h-full", className)}
        label={payloads[0].jsonLabel}
        value={payloads[0].json}
      />
    );
  }

  return (
    <div className={cn("flex h-full min-h-0 flex-col gap-3 overflow-hidden", className)}>
      {payloads.map((payload, index) => (
        <SessionJsonPanel
          key={`${payload.jsonLabel}-${index}`}
          className="min-h-0 flex-1"
          label={payload.jsonLabel}
          value={payload.json}
        />
      ))}
    </div>
  );
}
