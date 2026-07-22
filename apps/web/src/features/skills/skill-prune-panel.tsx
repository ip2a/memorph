import { useState } from "react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import { useExecuteSkillPrune, useSkillPrune } from "@/features/skills/queries";
import { formatBytes } from "@/lib/format";

export function SkillPrunePanel() {
  const [days, setDays] = useState(30);
  const [selected, setSelected] = useState<string[]>([]);
  const preview = useSkillPrune(days);
  const execute = useExecuteSkillPrune();
  return (
    <PanelCard className="space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SectionHeading title="安全清理" className="border-0 pb-0" />
        <div className="flex gap-1">
          {[30, 60, 90].map((value) => (
            <Button
              key={value}
              size="sm"
              variant={days === value ? "default" : "outline"}
              onClick={() => {
                setDays(value);
                setSelected([]);
              }}
            >
              {value} 天
            </Button>
          ))}
        </div>
      </div>
      <Alert>
        <AlertTitle>真实目录永不删除</AlertTitle>
        <AlertDescription>
          Prune 只会移除安全符号链接或带有效 Memorph
          标记的受管复制；历史不完整或路径变化时后端会拒绝执行。
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
                {item.metadata_tokens} metadata tokens
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
                toast.success(`已安全移除 ${selected.length} 个安装`);
                setSelected([]);
              },
              onError: (error) => toast.error(error.message),
            },
          )
        }
      >
        确认移除 {selected.length ? `${selected.length} 项` : "所选项"}
      </Button>
    </PanelCard>
  );
}
