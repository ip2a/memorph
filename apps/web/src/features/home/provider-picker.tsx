import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/shared/provider-logo";
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
            className="rounded-full"
            onClick={() => onToggle(provider.provider_id)}
          >
            <ProviderLogo
              providerId={provider.provider_id}
              size="xs"
              alt={provider.display_name || provider.provider_id}
            />
            {provider.display_name || provider.provider_id}
          </Button>
        );
      })}
    </div>
  );
}
