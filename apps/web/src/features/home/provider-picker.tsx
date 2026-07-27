import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { cn } from "@/lib/utils";
import type { ProviderCatalogEntry } from "@/lib/types";

type ProviderPickerProps = {
  candidates: ProviderCatalogEntry[];
  selected: string[];
  onToggle: (providerId: string) => void;
};

export function ProviderPicker({ candidates, selected, onToggle }: ProviderPickerProps) {
  return (
    <div className="flex max-h-56 flex-wrap gap-2 overflow-y-auto">
      {candidates.map((provider) => {
        const checked = selected.includes(provider.provider_id);
        return (
          <Button
            key={provider.provider_id}
            type="button"
            variant="outline"
            size="sm"
            className={cn(
              "rounded-full",
              checked &&
                "border-primary/50 bg-primary/10 text-foreground hover:bg-primary/15 dark:bg-primary/15 dark:hover:bg-primary/20",
            )}
            aria-pressed={checked}
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
