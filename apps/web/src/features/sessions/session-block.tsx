import {
  BoxIcon,
  BrainIcon,
  CodeIcon,
  FileIcon,
  FileTextIcon,
  ImageIcon,
  PackageIcon,
  TerminalIcon,
  WrenchIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import type { EventBlock } from "@/lib/types";

function formatJson(value: unknown) {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function CodeBlock({ value }: { value: unknown }) {
  const text = formatJson(value);
  if (!text) return <span className="text-muted-foreground">-</span>;

  return (
    <ScrollArea className="max-h-80 rounded-md bg-muted">
      <pre className="whitespace-pre-wrap break-words p-3 font-mono text-xs">{text}</pre>
    </ScrollArea>
  );
}

function BlockCard({
  icon,
  title,
  description,
  children,
}: {
  icon: ReactNode;
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {icon}
          {title}
        </CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

function FileList({ files }: { files: string[] | undefined }) {
  if (!files || files.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-2">
      {files.map((file) => (
        <Badge key={file} variant="outline">
          {file}
        </Badge>
      ))}
    </div>
  );
}

function imageSource(block: Extract<EventBlock, { type: "image" }>) {
  if (!block.data) return null;
  if (block.data.startsWith("data:")) return block.data;
  return `data:${block.mime_type};base64,${block.data}`;
}

export function SessionBlock({ block }: { block: EventBlock }) {
  switch (block.type) {
    case "text":
      return (
        <BlockCard icon={<FileTextIcon />} title="Text">
          <CodeBlock value={block.text} />
        </BlockCard>
      );
    case "thinking":
      return (
        <BlockCard
          icon={<BrainIcon />}
          title="Thinking"
          description={block.signature ? `Signature: ${block.signature}` : undefined}
        >
          <CodeBlock value={block.text} />
        </BlockCard>
      );
    case "tool_call":
      return (
        <BlockCard
          icon={<WrenchIcon />}
          title={block.name}
          description={block.tool_call_id}
        >
          <CodeBlock value={block.input} />
        </BlockCard>
      );
    case "tool_result":
      return (
        <BlockCard
          icon={<BoxIcon />}
          title="Tool Result"
          description={
            <span className="inline-flex items-center gap-2">
              <span>{block.tool_call_id}</span>
              {block.is_error ? <Badge variant="destructive">Error</Badge> : <Badge variant="secondary">OK</Badge>}
            </span>
          }
        >
          <CodeBlock value={block.content} />
        </BlockCard>
      );
    case "patch":
      return (
        <BlockCard
          icon={<CodeIcon />}
          title="Patch"
          description={block.hash ? `Hash: ${block.hash}` : undefined}
        >
          <div className="flex flex-col gap-3">
            {block.summary ? <p>{block.summary}</p> : null}
            <FileList files={block.files} />
            {block.diff_text ? <CodeBlock value={block.diff_text} /> : null}
          </div>
        </BlockCard>
      );
    case "command":
      return (
        <BlockCard icon={<TerminalIcon />} title="Command" description={block.cwd ?? undefined}>
          <div className="flex flex-col gap-3">
            <CodeBlock value={block.command} />
            {block.argv && block.argv.length > 0 ? <CodeBlock value={block.argv} /> : null}
          </div>
        </BlockCard>
      );
    case "command_result":
      return (
        <BlockCard
          icon={<TerminalIcon />}
          title="Command Result"
          description={block.exit_code === null || block.exit_code === undefined ? undefined : `Exit ${block.exit_code}`}
        >
          <div className="flex flex-col gap-3">
            {block.command ? <CodeBlock value={block.command} /> : null}
            {block.stdout ? <CodeBlock value={block.stdout} /> : null}
            {block.stderr ? (
              <>
                <Separator />
                <CodeBlock value={block.stderr} />
              </>
            ) : null}
          </div>
        </BlockCard>
      );
    case "file":
      return (
        <BlockCard icon={<FileIcon />} title={block.path} description={block.mime_type ?? undefined}>
          <CodeBlock value={block.content} />
        </BlockCard>
      );
    case "image": {
      const src = imageSource(block);
      return (
        <BlockCard icon={<ImageIcon />} title="Image" description={block.path ?? block.mime_type}>
          {src ? <img alt={block.path ?? "Session image"} className="max-h-96 rounded-md object-contain" src={src} /> : <CodeBlock value={block} />}
        </BlockCard>
      );
    }
    case "provider_payload":
      return (
        <BlockCard icon={<PackageIcon />} title="Provider Payload" description={block.kind}>
          <CodeBlock value={block.payload} />
        </BlockCard>
      );
    case "compressed":
      return (
        <BlockCard icon={<PackageIcon />} title="Compressed" description={block.archive_ref ?? block.source_provider_id}>
          <div className="flex flex-col gap-3">
            <p>{block.summary}</p>
            {block.source_event_count === null || block.source_event_count === undefined ? null : (
              <Badge variant="secondary">{block.source_event_count} source events</Badge>
            )}
            <FileList files={block.source_event_ids} />
          </div>
        </BlockCard>
      );
    case "unknown":
      return (
        <BlockCard icon={<BoxIcon />} title="Unknown">
          <CodeBlock value={block.raw} />
        </BlockCard>
      );
    default: {
      const unknownBlock = block as { type?: string } & Record<string, unknown>;
      return (
        <BlockCard icon={<BoxIcon />} title={unknownBlock.type || "Block"}>
          <CodeBlock value={unknownBlock} />
        </BlockCard>
      );
    }
  }
}
