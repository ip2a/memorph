import { CopyIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { useSkillCoverage } from "@/features/skills/queries";
import { formatBytes } from "@/lib/format";
import { renderHighlightedJson } from "@/lib/format-content";
import type { SkillAsset, SkillFilePreview } from "@/lib/types";
import { cn } from "@/lib/utils";

const BUNDLE_SHELL_HEIGHT = "h-[calc(15rem+5rem)]";
const CARD_CLASS =
  "flex h-full min-h-0 w-full flex-col gap-0 overflow-hidden rounded-xl border border-border bg-card py-0 shadow-none ring-0";

const IMAGE_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "svg",
  "ico",
  "bmp",
]);

function basename(path: string) {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

function isImageAsset(asset?: SkillAsset, preview?: SkillFilePreview | null) {
  if (preview?.encoding === "base64" && preview.mime_type?.startsWith("image/")) {
    return true;
  }
  const ext = (preview?.extension ?? asset?.extension ?? "").toLowerCase();
  return IMAGE_EXTENSIONS.has(ext);
}

function imageDataUrl(preview: SkillFilePreview) {
  const mime = preview.mime_type || "application/octet-stream";
  return `data:${mime};base64,${preview.content}`;
}

async function copyContent(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success("Copied");
  } catch {
    toast.error("Failed to copy");
  }
}

function BundleFileList({
  assets,
  previewPath,
  onSelect,
  coverage,
}: {
  assets: SkillAsset[];
  previewPath: string | null;
  onSelect: (path: string) => void;
  coverage: Map<string, { observations: number; confidence?: string | null }>;
}) {
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto border-y border-border">
        {assets.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">No bundle files.</p>
        ) : (
          assets.map((asset) => {
            const selected = previewPath === asset.path;
            const status = coverage.get(asset.path);
            return (
              <button
                key={asset.path}
                type="button"
                onClick={() => onSelect(asset.path)}
                className={cn(
                  "flex w-full items-center justify-between gap-3 border-b border-border px-2 py-2 text-left transition-colors last:border-b-0 hover:bg-muted/50",
                  selected && "bg-secondary hover:bg-secondary",
                )}
              >
                <span className="min-w-0 truncate font-mono text-sm font-medium">
                  {basename(asset.path)}
                </span>
                <span className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
                  <Badge variant={status?.confidence ? "outline" : "secondary"}>
                    {status
                      ? `${status.observations} · ${status.confidence ?? "未覆盖"}`
                      : "未覆盖"}
                  </Badge>
                  {formatBytes(asset.bytes)}
                </span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}

function BundlePreviewPanel({
  path,
  preview,
  asset,
  loading,
}: {
  path: string | null;
  preview?: SkillFilePreview | null;
  asset?: SkillAsset;
  loading?: boolean;
}) {
  const text = preview?.encoding === "text" ? (preview.content ?? "") : "";
  const highlighted = text ? renderHighlightedJson(text) : null;
  const showImage = Boolean(preview && isImageAsset(asset, preview));

  return (
    <Card className={CARD_CLASS} size="sm">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b px-4 py-2">
        <h4 className="truncate font-mono text-sm font-semibold tracking-tight">
          {path ? basename(path) : "Preview"}
        </h4>
        <div className="flex shrink-0 items-center gap-2">
          {asset ? <Badge variant="outline">{asset.category}</Badge> : null}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 px-2 text-xs text-muted-foreground"
            disabled={!text}
            onClick={() => copyContent(text)}
          >
            <CopyIcon data-icon="inline-start" />
            Copy
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <div className={cn("p-4", showImage && "flex h-full items-center justify-center")}>
          {loading ? (
            <div className="flex h-full items-center justify-center">
              <Spinner />
            </div>
          ) : !path ? (
            <p className="text-sm text-muted-foreground">Select a file to preview.</p>
          ) : !asset?.previewable ? (
            <p className="text-sm text-muted-foreground">This file is not previewable.</p>
          ) : showImage && preview ? (
            <img
              alt={basename(path)}
              className="max-h-full max-w-full rounded-md object-contain"
              src={imageDataUrl(preview)}
            />
          ) : highlighted ? (
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
      </div>

      <div className="shrink-0 border-t px-4 py-2 font-mono text-xs text-muted-foreground">
        {asset ? `${asset.path} · ${formatBytes(asset.bytes)}` : "File preview"}
      </div>
    </Card>
  );
}

export function SkillBundlePanel({
  skillId,
  assets,
  previewPath,
  onPreviewPathChange,
  preview,
  previewLoading,
}: {
  skillId: string | null;
  assets: SkillAsset[];
  previewPath: string | null;
  onPreviewPathChange: (path: string) => void;
  preview?: SkillFilePreview | null;
  previewLoading?: boolean;
}) {
  const selectedAsset = assets.find((asset) => asset.path === previewPath);
  const coverage = useSkillCoverage(skillId, "90d");
  const coverageByPath = new Map(
    (coverage.data?.targets ?? [])
      .filter((target) => target.target_path)
      .map((target) => [target.target_path as string, target]),
  );

  return (
    <div
      className={cn(
        "grid w-full min-w-0 grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-4",
        BUNDLE_SHELL_HEIGHT,
      )}
    >
      <div className="h-full min-h-0 overflow-hidden">
        <BundleFileList
          assets={assets}
          previewPath={previewPath}
          onSelect={onPreviewPathChange}
          coverage={coverageByPath}
        />
      </div>
      <div className="h-full min-h-0 overflow-hidden">
        <BundlePreviewPanel
          path={previewPath}
          preview={preview}
          asset={selectedAsset}
          loading={previewLoading}
        />
      </div>
    </div>
  );
}
