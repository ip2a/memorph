import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import { useExecuteSkillPrune, useSkillPrune } from "@/features/skills/queries";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";

const PRUNE_DAY_OPTIONS = [30, 60, 90] as const;

export function SkillPruneDayTabs({
  days,
  onDaysChange,
  className,
}: {
  days: number;
  onDaysChange: (days: number) => void;
  className?: string;
}) {
  const { t } = useI18n();
  return (
    <Tabs
      value={String(days)}
      onValueChange={(value) => onDaysChange(Number(value))}
      className={className}
    >
      <TabsList aria-label={t("skillsCleanupRange")}>
        {PRUNE_DAY_OPTIONS.map((value) => (
          <TabsTrigger key={value} value={String(value)}>
            {t("skillsDays", { count: value })}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}

export function SkillPrunePanel({
  embedded = false,
  days: daysProp,
  onDaysChange,
}: {
  embedded?: boolean;
  days?: number;
  onDaysChange?: (days: number) => void;
} = {}) {
  const { t } = useI18n();
  const [internalDays, setInternalDays] = useState(30);
  const days = daysProp ?? internalDays;
  const setDays = onDaysChange ?? setInternalDays;
  const [selected, setSelected] = useState<string[]>([]);
  const preview = useSkillPrune(days);
  const execute = useExecuteSkillPrune();

  useEffect(() => {
    setSelected([]);
  }, [days]);

  const content = (
    <>
      {!embedded ? (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SectionHeading title={t("skillsSafePrune")} className="border-0 pb-0" />
          <SkillPruneDayTabs days={days} onDaysChange={setDays} />
        </div>
      ) : null}
      <Alert>
        <AlertTitle>{t("skillsRealDirectoriesSafe")}</AlertTitle>
        <AlertDescription>
          {t("skillsPruneSafetyHint")}
        </AlertDescription>
      </Alert>
      {preview.data?.blocked_reason && (
        <p className="text-sm text-destructive">
          {preview.data.blocked_reason}
        </p>
      )}
      <div className="max-h-56 space-y-2 overflow-auto">
        {preview.data?.items.map((item) => (
          <label
            key={item.installation_id}
            className="flex items-start gap-3 rounded-md border p-3 text-sm"
          >
            <Checkbox
              disabled={!item.executable}
              checked={selected.includes(item.installation_id)}
              onCheckedChange={(checked) =>
                setSelected((current) =>
                  checked
                    ? [...current, item.installation_id]
                    : current.filter((id) => id !== item.installation_id),
                )
              }
            />
            <span className="min-w-0 flex-1">
              <span className="font-medium">{item.name}</span>
              <span className="block truncate text-muted-foreground">
                {item.install_path}
              </span>
              <span className="text-muted-foreground">
                {item.install_kind} · {formatBytes(item.installation_bytes)} ·{" "}
                {item.metadata_tokens} {t("skillsTokens")}
              </span>
              {item.blocked_reason && (
                <span className="block text-destructive">
                  {item.blocked_reason}
                </span>
              )}
            </span>
          </label>
        ))}
      </div>
      <Button
        disabled={!preview.data || selected.length === 0 || execute.isPending}
        onClick={() =>
          preview.data &&
          execute.mutate(
            { preview: preview.data, installationIds: selected },
            {
              onSuccess: () => {
                toast.success(t("skillsPruned", { count: selected.length }));
                setSelected([]);
              },
              onError: (error) => toast.error(error.message),
            },
          )
        }
      >
        {selected.length
          ? t("skillsSelectedItems", { count: selected.length })
          : t("skillsSelectedItemFallback")}
      </Button>
    </>
  );

  if (embedded) {
    return <div className="space-y-3">{content}</div>;
  }

  return <PanelCard className="space-y-3 p-4">{content}</PanelCard>;
}
