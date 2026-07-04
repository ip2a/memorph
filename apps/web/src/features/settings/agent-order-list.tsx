import { ArrowDownIcon, ArrowUpIcon, EyeIcon, EyeOffIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SortableItem, SortableItemHandle, SortableList } from "@/components/shared/sortable-list";
import type { I18nKey } from "@/lib/i18n-core";
import type { ProviderCatalogEntry } from "@/lib/types";
import { cn } from "@/lib/utils";

type AgentOrderListProps = {
  orderedProviderIds: string[];
  providerMap: Map<string, ProviderCatalogEntry>;
  hiddenAgents: string[];
  onReorder: (next: string[]) => void;
  onHiddenChange: (id: string, hidden: boolean) => void;
  onShift: (index: number, direction: "up" | "down") => void;
  t: (key: I18nKey, vars?: Record<string, string | number | null | undefined>) => string;
};

function providerName(provider: ProviderCatalogEntry | undefined, id: string) {
  return provider?.display_name || id;
}

function providerInstalled(provider: ProviderCatalogEntry | undefined) {
  return Boolean(provider?.install_state?.is_installed || provider?.filter_tags?.includes("is_installed"));
}

function AgentOrderRow({
  id,
  index,
  total,
  provider,
  hidden,
  onHiddenChange,
  onShift,
  t,
  overlay = false,
}: {
  id: string;
  index: number;
  total: number;
  provider: ProviderCatalogEntry | undefined;
  hidden: boolean;
  onHiddenChange: (id: string, hidden: boolean) => void;
  onShift: (index: number, direction: "up" | "down") => void;
  t: AgentOrderListProps["t"];
  overlay?: boolean;
}) {
  return (
    <div
      className={cn(
        "grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 rounded-md border p-3",
        hidden ? "opacity-70" : "",
      )}
    >
      {!overlay ? (
        <SortableItemHandle label={t("dragToReorder")} />
      ) : (
        <div className="size-7" aria-hidden />
      )}
      <div className="flex min-w-0 items-center gap-2">
        <strong className={cn(hidden ? "line-through" : "")}>{providerName(provider, id)}</strong>
        <Badge variant={providerInstalled(provider) ? "secondary" : "outline"}>
          {providerInstalled(provider) ? t("installed") : t("notDetected")}
        </Badge>
      </div>
      <div className="flex items-center justify-end gap-1">
        {!overlay ? (
          <>
            <Button
              type="button"
              variant={hidden ? "secondary" : "ghost"}
              size="icon-sm"
              aria-label={t("hidden")}
              aria-pressed={hidden}
              onClick={() => onHiddenChange(id, !hidden)}
            >
              {hidden ? <EyeOffIcon /> : <EyeIcon />}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={index === 0}
              aria-label={t("moveUp")}
              onClick={() => onShift(index, "up")}
            >
              <ArrowUpIcon />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={index >= total - 1}
              aria-label={t("moveDown")}
              onClick={() => onShift(index, "down")}
            >
              <ArrowDownIcon />
            </Button>
          </>
        ) : null}
      </div>
    </div>
  );
}

export function AgentOrderList({
  orderedProviderIds,
  providerMap,
  hiddenAgents,
  onReorder,
  onHiddenChange,
  onShift,
  t,
}: AgentOrderListProps) {
  const items = orderedProviderIds.map((id) => ({ id }));

  return (
    <SortableList
      items={items}
      className="flex flex-col gap-2"
      onReorder={(next) => onReorder(next.map((item) => item.id))}
      renderOverlay={(item) => {
        const index = orderedProviderIds.indexOf(item.id);
        return (
          <AgentOrderRow
            id={item.id}
            index={index}
            total={orderedProviderIds.length}
            provider={providerMap.get(item.id)}
            hidden={hiddenAgents.includes(item.id)}
            onHiddenChange={onHiddenChange}
            onShift={onShift}
            t={t}
            overlay
          />
        );
      }}
    >
      {orderedProviderIds.map((id, index) => (
        <SortableItem key={id} id={id}>
          <AgentOrderRow
            id={id}
            index={index}
            total={orderedProviderIds.length}
            provider={providerMap.get(id)}
            hidden={hiddenAgents.includes(id)}
            onHiddenChange={onHiddenChange}
            onShift={onShift}
            t={t}
          />
        </SortableItem>
      ))}
    </SortableList>
  );
}
