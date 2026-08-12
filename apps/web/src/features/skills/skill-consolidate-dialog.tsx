import { useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { Spinner } from "@/components/ui/spinner";
import { useSkillGroupInstallations } from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";

function installKindBadgeKey(
  deployment_mode: string,
): I18nKey {
  if (deployment_mode === "symlink") return "skillsInstallKindSymlink";
  if (deployment_mode === "copy") return "skillsInstallKindManagedCopy";
  return "skillsInstallKindDirectory";
}

export function SkillConsolidateDialog({
  open,
  sourceId,
  skillName,
  pending,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  sourceId: string | null;
  skillName: string;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (canonicalPath: string) => void;
}) {
  const { t } = useI18n();
  const groupQuery = useSkillGroupInstallations(open ? sourceId : null);
  const installations = groupQuery.data?.installations ?? [];
  const [canonicalPath, setCanonicalPath] = useState<string | null>(null);
  const canonicalFingerprint =
    installations.find((installation) => installation.path === canonicalPath)
      ?.fingerprint ?? null;

  useEffect(() => {
    if (!open) {
      setCanonicalPath(null);
      return;
    }
    // Default to the first real directory (managed or external), which is the
    // most natural canonical — symlinks resolve to it anyway.
    const real = installations.find(
      (installation) => installation.deployment_mode !== "symlink",
    );
    setCanonicalPath(real?.path ?? installations[0]?.path ?? null);
  }, [open, installations]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(80dvh,640px)] w-[min(640px,calc(100vw-2rem))] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <DialogHeader className="shrink-0 border-b px-6 py-4">
          <DialogTitle>{t("skillsConsolidateTitle")}</DialogTitle>
          <DialogDescription>{t("skillsConsolidateDescription")}</DialogDescription>
        </DialogHeader>
        <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-2 px-6 py-4">
          {groupQuery.isLoading ? (
            <div className="flex justify-center py-8">
              <Spinner />
            </div>
          ) : installations.length < 2 ? (
            <p className="text-muted-foreground text-sm">
              {t("skillsConsolidateHint")}
            </p>
          ) : (
            installations.map((installation) => {
              const drifted =
                canonicalFingerprint != null &&
                installation.fingerprint !== canonicalFingerprint;
              const selected = installation.path === canonicalPath;
              return (
                <SelectableRowButton
                  key={installation.path}
                  type="button"
                  selected={selected}
                  title={<strong className="text-sm">{installation.used_by}</strong>}
                  meta={installation.path}
                  onClick={() => setCanonicalPath(installation.path)}
                  trailing={
                    <span className="flex shrink-0 flex-col items-end gap-1">
                      <Badge variant="outline">
                        {t(installKindBadgeKey(installation.deployment_mode))}
                      </Badge>
                      {drifted && !selected ? (
                        <span className="text-destructive max-w-[12rem] text-right text-[11px] leading-tight">
                          {t("skillsConsolidateConflictHint")}
                        </span>
                      ) : null}
                    </span>
                  }
                />
              );
            })
          )}
        </ScrollPane>
        <DialogFooter className="shrink-0 border-t px-6 py-4">
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={pending}
          >
            {t("cancel")}
          </Button>
          <Button
            disabled={pending || !canonicalPath}
            onClick={() => {
              if (canonicalPath) onConfirm(canonicalPath);
            }}
          >
            {pending ? <Spinner data-icon="inline-start" /> : null}
            {t("skillsConsolidateConfirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
