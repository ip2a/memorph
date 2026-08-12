import { ArchiveIcon, RotateCcwIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { Spinner } from "@/components/ui/spinner";
import { useDisabledSkills, useEnableSkill } from "@/features/skills/queries";
import { normalizeSkillDescription } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import { toast } from "sonner";

export function SkillDisabledDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useI18n();
  const disabledQuery = useDisabledSkills();
  const enableMutation = useEnableSkill();
  const items = disabledQuery.data?.items ?? [];
  const enabling = enableMutation.variables?.directory ?? null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(80dvh,640px)] w-[min(640px,calc(100vw-2rem))] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <DialogHeader className="shrink-0 border-b px-6 py-4">
          <DialogTitle>{t("skillsDisabledList")}</DialogTitle>
          <DialogDescription>{t("skillsDisabledListDescription")}</DialogDescription>
        </DialogHeader>
        <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-2 px-6 py-4">
          {disabledQuery.isLoading ? (
            <div className="flex justify-center py-8">
              <Spinner />
            </div>
          ) : items.length === 0 ? (
            <Empty className="border border-dashed">
              <EmptyHeader>
                <EmptyTitle>{t("skillsDisabledEmpty")}</EmptyTitle>
                <EmptyDescription>{t("skillsDisabledListDescription")}</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            items.map((item) => {
              const pending = enabling === item.directory;
              return (
                <SelectableRowButton
                  key={item.archive_path}
                  type="button"
                  selected={false}
                  leading={<ArchiveIcon className="text-muted-foreground size-4 shrink-0" />}
                  title={<strong className="text-sm">{item.name}</strong>}
                  meta={
                    <span className="flex flex-col gap-0.5">
                      <span className="flex items-center gap-1">
                        <Badge variant="outline">{item.used_by}</Badge>
                        <span className="truncate">{normalizeSkillDescription(item.description) || item.directory}</span>
                      </span>
                      <span className="truncate font-mono text-[11px]">{item.archive_path}</span>
                    </span>
                  }
                  trailing={
                    <Button
                      size="sm"
                      className="shrink-0"
                      disabled={enableMutation.isPending}
                      onClick={() => {
                        enableMutation.mutate(
                          { usedBy: item.used_by, directory: item.directory },
                          {
                            onSuccess: () =>
                              toast.success(t("skillsEnabledRestored", { skill: item.name })),
                          },
                        );
                      }}
                    >
                      {pending ? <Spinner data-icon="inline-start" /> : <RotateCcwIcon />}
                      {t("skillsEnableRestore")}
                    </Button>
                  }
                />
              );
            })
          )}
        </ScrollPane>
      </DialogContent>
    </Dialog>
  );
}
