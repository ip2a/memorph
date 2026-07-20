import { providerInstalled } from "@/features/home/model/providers";
import type { ProviderCatalogEntry } from "@/lib/types";

export type ManagerProviderOption = {
  id: string;
  name: string;
};

function providerScannable(entry: ProviderCatalogEntry) {
  return Boolean(entry.capability_set?.scan);
}

function providerVisibleInManager(entry: ProviderCatalogEntry) {
  return !entry.hidden_state?.global;
}

export function managerProviderCandidates(catalog: ProviderCatalogEntry[]) {
  return catalog.filter(
    (item) =>
      providerVisibleInManager(item) &&
      providerInstalled(item) &&
      providerScannable(item),
  );
}

export function managerProviderOptions(
  catalog: ProviderCatalogEntry[],
): ManagerProviderOption[] {
  return managerProviderCandidates(catalog).map((item) => ({
    id: item.provider_id,
    name: item.display_name || item.provider_id,
  }));
}
