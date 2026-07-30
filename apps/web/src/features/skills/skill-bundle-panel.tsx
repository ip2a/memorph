import { CopyIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { formatBytes } from "@/lib/format";
import { renderHighlightedJson } from "@/lib/format-content";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAsset, SkillFilePreview } from "@/lib/types";
import { cn } from "@/lib/utils";

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

async function copyContent(text: string, copied: string, failed: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(copied);
  } catch {
    toast.error(failed);
  }
}

function fileTypeLabel(asset: SkillAsset) {
  const ext = asset.extension?.replace(/^\./, "").trim();
  if (ext) return `.${ext.toLowerCase()}`;
  return asset.category;
}

function categoryLabel(
  category: SkillAsset["category"],
  t: (key: import("@/lib/i18n-core").I18nKey) => string,
) {
  switch (category) {
    case "entry":
      return t("skillsAssetCategoryEntry");
    case "script":
      return t("skillsAssetCategoryScript");
    case "reference":
      return t("skillsAssetCategoryReference");
    case "asset":
      return t("skillsAssetCategoryAsset");
    case "metadata":
      return t("skillsAssetCategoryMetadata");
    default:
      return t("skillsAssetCategoryOther");
  }
}

function BundleFileList({
  assets,
  previewPath,
  onSelect,
}: {
  assets: SkillAsset[];
  previewPath: string | null;
  onSelect: (path: string) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1">
        {assets.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">{t("skillsNoBundleFiles")}</p>
        ) : (
          assets.map((asset) => {
            const selected = previewPath === asset.path;
            return (
              <SelectableRowButton
                key={asset.path}
                selected={selected}
                className="[&_strong]:font-mono"
                title={basename(asset.path)}
                details={
                  <span className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">{fileTypeLabel(asset)}</Badge>
                    <Badge variant="secondary">
                      {categoryLabel(asset.category, t)}
                    </Badge>
                    <span className="text-xs text-muted-foreground">
                      {formatBytes(asset.bytes)}
                    </span>
                  </span>
                }
                onClick={() => onSelect(asset.path)}
              />
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
  const { t } = useI18n();
  const text = preview?.encoding === "text" ? (preview.content ?? "") : "";
  const highlighted = text ? renderHighlightedJson(text) : null;
  const showImage = Boolean(preview && isImageAsset(asset, preview));

  return (
    <Card className={CARD_CLASS} size="sm">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b px-4 py-2">
        <h4 className="truncate font-mono text-sm font-semibold tracking-tight">
          {path ? basename(path) : t("skillsPreview")}
        </h4>
        <div className="flex shrink-0 items-center gap-2">
          {asset ? <Badge variant="outline">{asset.category}</Badge> : null}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 px-2 text-xs text-muted-foreground"
            disabled={!text}
            onClick={() => copyContent(text, t("skillsCopied"), t("skillsCopyFailed"))}
          >
            <CopyIcon data-icon="inline-start" />
            {t("skillsCopy")}
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
            <p className="text-sm text-muted-foreground">{t("skillsSelectFilePreview")}</p>
          ) : !asset?.previewable ? (
            <p className="text-sm text-muted-foreground">{t("skillsFileNotPreviewable")}</p>
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
  assets,
  previewPath,
  onPreviewPathChange,
  preview,
  previewLoading,
  className,
}: {
  assets: SkillAsset[];
  previewPath: string | null;
  onPreviewPathChange: (path: string) => void;
  preview?: SkillFilePreview | null;
  previewLoading?: boolean;
  className?: string;
}) {
  const selectedAsset = assets.find((asset) => asset.path === previewPath);

  return (
    <div
      className={cn(
        "grid h-full min-h-0 w-full min-w-0 flex-1 grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-4",
        className,
      )}
    >
      <div className="h-full min-h-0 overflow-hidden">
        <BundleFileList
          assets={assets}
          previewPath={previewPath}
          onSelect={onPreviewPathChange}
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
