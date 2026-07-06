import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import { orderProviderPills } from "@/features/home/model/providers";
import type { ProviderCatalogEntry } from "@/lib/types";

type ProviderPickerProps = {
  candidates: ProviderCatalogEntry[];
  selected: string[];
  onToggle: (providerId: string) => void;
};

export function ProviderPicker({ candidates, selected, onToggle }: ProviderPickerProps) {
  const ordered = useMemo(() => orderProviderPills(candidates, selected), [candidates, selected]);

  return (
    <div className="flex max-h-56 flex-wrap gap-2 overflow-y-auto">
      {ordered.map((provider) => {
        const checked = selected.includes(provider.provider_id);
        return (
          <Button
            key={provider.provider_id}
            type="button"
            variant={checked ? "default" : "outline"}
            size="sm"
            className="rounded-full font-mono"
            onClick={() => onToggle(provider.provider_id)}
          >
            {provider.display_name || provider.provider_id}
          </Button>
        );
      })}
    </div>
  );
}
