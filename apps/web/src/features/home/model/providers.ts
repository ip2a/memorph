import type { ProviderCatalogEntry } from "@/lib/types";

function providerInstalled(entry: ProviderCatalogEntry) {
  return Boolean(entry.install_state?.is_installed || entry.filter_tags?.includes("is_installed"));
}

export function homeProviderCandidates(catalog: ProviderCatalogEntry[]) {
  const visible = catalog.filter((item) => !item.hidden_state?.global && providerInstalled(item));
  if (visible.length) return visible;

  const fallbackIds = ["claude", "codex"];
  const fallback = fallbackIds
    .map((id) => catalog.find((item) => item.provider_id === id))
    .filter((item): item is ProviderCatalogEntry => Boolean(item));
  return fallback.length ? fallback : catalog.slice(0, 2);
}

export function resolveHomeProviders(candidates: ProviderCatalogEntry[], savedProviders: string[] | undefined) {
  const candidateIds = candidates.map((item) => item.provider_id);
  if (!candidateIds.length) return [];
  if (!savedProviders?.length) return candidateIds;

  const selected = savedProviders.filter((id) => candidateIds.includes(id));
  return selected.length ? selected : candidateIds;
}

export function orderProviderPills(candidates: ProviderCatalogEntry[], selected: string[]) {
  if (!selected.length) return candidates;

  const selectedSet = new Set(selected);
  const prioritized: ProviderCatalogEntry[] = [];
  const remainder: ProviderCatalogEntry[] = [];

  for (const candidate of candidates) {
    if (selectedSet.has(candidate.provider_id)) {
      prioritized.push(candidate);
    } else {
      remainder.push(candidate);
    }
  }

  return [...prioritized, ...remainder];
}
