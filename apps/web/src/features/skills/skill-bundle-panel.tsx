import { useEffect, useState } from "react";
import {
  CheckIcon,
  CopyIcon,
  EyeIcon,
  FileCode2Icon,
  Maximize2Icon,
  PencilIcon,
  Undo2Icon,
  XIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { MarkdownContent } from "@/components/shared/markdown-content";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { useUpdateSkillFile } from "@/features/skills/queries";
import { formatBytes } from "@/lib/format";
import {
  detectSessionContentKind,
  renderHighlightedJson,
} from "@/lib/format-content";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAsset, SkillFilePreview } from "@/lib/types";
import { cn } from "@/lib/utils";

const CARD_CLASS =
  "flex h-full min-h-0 w-full flex-col gap-0 overflow-hidden rounded-xl border border-border bg-card py-0 shadow-none ring-0";

const FULLSCREEN_DIALOG_CLASS =
  "h-[min(95dvh,calc(100dvh-2rem))] w-[min(calc(100vw-2rem),1400px)] max-w-[calc(100vw-2rem)] sm:max-w-none";

const BUNDLE_CHROME_CLASS =
  "flex shrink-0 items-center justify-between gap-3 border-b px-4 py-4 sm:px-5";

const BUNDLE_FOOTER_CLASS =
  "shrink-0 border-t px-4 py-4 font-mono text-xs text-muted-foreground sm:px-5";

const MARKDOWN_EXTENSIONS = new Set(["md", "markdown", "mdx"]);

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

function fileExtension(
  path: string | null,
  asset?: SkillAsset,
  preview?: SkillFilePreview | null,
) {
  const fromMeta = (preview?.extension ?? asset?.extension ?? "")
    .toLowerCase()
    .replace(/^\./, "");
  if (fromMeta) return fromMeta;
  if (!path) return "";
  const parts = path.split(".");
  return parts.length > 1 ? (parts.at(-1)?.toLowerCase() ?? "") : "";
}

function isImageAsset(asset?: SkillAsset, preview?: SkillFilePreview | null) {
  if (preview?.encoding === "base64" && preview.mime_type?.startsWith("image/")) {
    return true;
  }
  return IMAGE_EXTENSIONS.has(fileExtension(null, asset, preview));
}

function isEditableText(
  asset?: SkillAsset,
  preview?: SkillFilePreview | null,
) {
  return Boolean(
    asset?.previewable &&
      preview?.encoding === "text" &&
      !isImageAsset(asset, preview),
  );
}

function isMarkdownFile(
  path: string | null,
  asset?: SkillAsset,
  preview?: SkillFilePreview | null,
  text = "",
) {
  if (MARKDOWN_EXTENSIONS.has(fileExtension(path, asset, preview))) {
    return true;
  }
  return detectSessionContentKind(text) === "markdown";
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

function BundlePreviewEditor({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <textarea
      value={value}
      onChange={(event) => onChange(event.target.value)}
      spellCheck={false}
      className="m-0 block w-full min-h-0 resize-none border-0 bg-transparent p-0 font-mono text-[13px] leading-6 whitespace-pre-wrap break-words text-foreground outline-none [field-sizing:content] [overflow-wrap:anywhere] focus-visible:ring-0"
    />
  );
}

function BundlePreviewContent({
  path,
  preview,
  asset,
  loading,
  editing,
  draft,
  viewSource,
  onDraftChange,
}: {
  path: string | null;
  preview?: SkillFilePreview | null;
  asset?: SkillAsset;
  loading?: boolean;
  editing?: boolean;
  draft?: string;
  viewSource?: boolean;
  onDraftChange?: (value: string) => void;
}) {
  const { t } = useI18n();
  const text = preview?.encoding === "text" ? (preview.content ?? "") : "";
  const highlighted = text ? renderHighlightedJson(text) : null;
  const showImage = Boolean(preview && isImageAsset(asset, preview));
  const isMarkdown = isMarkdownFile(path, asset, preview, text);

  return (
    <div className={cn("p-4", showImage && "flex h-full items-center justify-center")}>
      {loading ? (
        <div className="flex h-full items-center justify-center">
          <Spinner />
        </div>
      ) : !path ? (
        <p className="text-sm text-muted-foreground">{t("skillsSelectFilePreview")}</p>
      ) : !asset?.previewable ? (
        <p className="text-sm text-muted-foreground">{t("skillsFileNotPreviewable")}</p>
      ) : editing ? (
        <BundlePreviewEditor
          value={draft ?? ""}
          onChange={(value) => onDraftChange?.(value)}
        />
      ) : showImage && preview ? (
        <img
          alt={basename(path)}
          className="max-h-full max-w-full rounded-md object-contain"
          src={imageDataUrl(preview)}
        />
      ) : isMarkdown && !viewSource ? (
        <MarkdownContent value={text} />
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
  );
}

function BundlePreviewToolbar({
  path,
  asset,
  preview,
  text,
  draft,
  editing,
  saving,
  allowEdit,
  isMarkdown,
  viewSource,
  showFullscreenButton,
  showCloseButton,
  onFullscreen,
  onToggleView,
  onEdit,
  onSave,
  onCancelEdit,
}: {
  path: string | null;
  asset?: SkillAsset;
  preview?: SkillFilePreview | null;
  text: string;
  draft: string;
  editing: boolean;
  saving?: boolean;
  allowEdit?: boolean;
  isMarkdown: boolean;
  viewSource: boolean;
  showFullscreenButton?: boolean;
  showCloseButton?: boolean;
  onFullscreen?: () => void;
  onToggleView?: () => void;
  onEdit?: () => void;
  onSave?: () => void;
  onCancelEdit?: () => void;
}) {
  const { t } = useI18n();
  const canExpand = Boolean(path && asset?.previewable && !editing);
  const canEdit = allowEdit && isEditableText(asset, preview);
  const copyText = editing ? draft : text;

  return (
    <div className="flex h-7 shrink-0 items-center gap-2">
      {asset ? <Badge variant="outline">{asset.category}</Badge> : null}
      {showFullscreenButton ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="text-muted-foreground"
          disabled={!canExpand}
          aria-label={t("skillsFullscreen")}
          onClick={onFullscreen}
        >
          <Maximize2Icon />
        </Button>
      ) : null}
      {isMarkdown && !editing ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className={cn("text-muted-foreground", !viewSource && "text-foreground")}
          aria-label={viewSource ? t("skillsViewRendered") : t("skillsViewSource")}
          onClick={onToggleView}
        >
          {viewSource ? <EyeIcon /> : <FileCode2Icon />}
        </Button>
      ) : null}
      {allowEdit ? (
        editing ? (
          <>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              disabled={saving}
              aria-label={t("cancel")}
              onClick={onCancelEdit}
            >
              <Undo2Icon />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              disabled={saving}
              aria-label={t("save")}
              onClick={onSave}
            >
              {saving ? <Spinner /> : <CheckIcon />}
            </Button>
          </>
        ) : (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground"
            disabled={!canEdit}
            aria-label={t("skillsEdit")}
            onClick={onEdit}
          >
            <PencilIcon />
          </Button>
        )
      ) : null}
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground"
        disabled={!copyText}
        aria-label={t("skillsCopy")}
        onClick={() => copyContent(copyText, t("skillsCopied"), t("skillsCopyFailed"))}
      >
        <CopyIcon />
      </Button>
      {showCloseButton ? (
        <DialogClose asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground"
          >
            <XIcon />
            <span className="sr-only">{t("close")}</span>
          </Button>
        </DialogClose>
      ) : null}
    </div>
  );
}

function BundlePreviewPanel({
  skillId,
  usedBy,
  path,
  preview,
  asset,
  loading,
}: {
  skillId?: string | null;
  usedBy?: string;
  path: string | null;
  preview?: SkillFilePreview | null;
  asset?: SkillAsset;
  loading?: boolean;
}) {
  const { t } = useI18n();
  const [fullscreenOpen, setFullscreenOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [viewSource, setViewSource] = useState(false);
  const [draft, setDraft] = useState("");
  const saveMutation = useUpdateSkillFile();
  const text = preview?.encoding === "text" ? (preview.content ?? "") : "";
  const title = path ? basename(path) : t("skillsPreview");
  const isMarkdown = isMarkdownFile(path, asset, preview, text);

  useEffect(() => {
    setEditing(false);
    setViewSource(false);
    setDraft("");
  }, [path, preview?.content]);

  useEffect(() => {
    if (!fullscreenOpen) {
      setEditing(false);
      setDraft("");
    }
  }, [fullscreenOpen]);

  function startEditing() {
    setViewSource(true);
    setDraft(text);
    setEditing(true);
  }

  function cancelEditing() {
    setDraft(text);
    setEditing(false);
    setViewSource(false);
  }

  async function saveEditing() {
    if (!skillId || !path || draft === text) {
      setEditing(false);
      setViewSource(false);
      return;
    }
    try {
      await saveMutation.mutateAsync({
        skillId,
        path,
        content: draft,
        usedBy,
      });
      toast.success(t("skillsSaved"));
      setEditing(false);
      setViewSource(false);
    } catch {
      toast.error(t("skillsSaveFailed"));
    }
  }

  const contentProps = {
    path,
    preview,
    asset,
    loading,
    editing,
    draft,
    viewSource,
    onDraftChange: setDraft,
  };

  const toolbarProps = {
    path,
    asset,
    preview,
    text,
    draft,
    editing,
    saving: saveMutation.isPending,
    isMarkdown,
    viewSource,
    onToggleView: () => setViewSource((value) => !value),
    onEdit: startEditing,
    onSave: () => void saveEditing(),
    onCancelEdit: cancelEditing,
  };

  return (
    <>
      <Card className={CARD_CLASS} size="sm">
        <div className={BUNDLE_CHROME_CLASS}>
          <h4 className="truncate font-mono text-sm font-semibold tracking-tight">{title}</h4>
          <BundlePreviewToolbar
            {...toolbarProps}
            showFullscreenButton
            onFullscreen={() => setFullscreenOpen(true)}
          />
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          <BundlePreviewContent {...contentProps} />
        </div>

        <div className={BUNDLE_FOOTER_CLASS}>
          {asset ? `${asset.path} · ${formatBytes(asset.bytes)}` : "File preview"}
        </div>
      </Card>

      <Dialog open={fullscreenOpen} onOpenChange={setFullscreenOpen}>
        <DialogContent variant="panel" className={FULLSCREEN_DIALOG_CLASS} showCloseButton={false}>
          <DialogHeader variant="bordered" className="flex-row items-center justify-between gap-3 space-y-0">
            <DialogTitle className="min-w-0 truncate font-mono text-sm leading-none">
              {title}
            </DialogTitle>
            <BundlePreviewToolbar {...toolbarProps} allowEdit showCloseButton />
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-auto">
            <BundlePreviewContent {...contentProps} />
          </div>

          <div className={BUNDLE_FOOTER_CLASS}>
            {asset ? `${asset.path} · ${formatBytes(asset.bytes)}` : "File preview"}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function SkillBundlePanel({
  skillId,
  usedBy,
  assets,
  previewPath,
  onPreviewPathChange,
  preview,
  previewLoading,
  className,
}: {
  skillId?: string | null;
  usedBy?: string;
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
          skillId={skillId}
          usedBy={usedBy}
          path={previewPath}
          preview={preview}
          asset={selectedAsset}
          loading={previewLoading}
        />
      </div>
    </div>
  );
}
