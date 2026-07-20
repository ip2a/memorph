import manifest from "@/assets/provider-logos/manifest.json";

export const PROVIDER_LOGO_MAPPING = manifest.memorph_mapping as Record<string, string>;

export const PROVIDER_LOGO_ASSET_IDS = manifest.providers as string[];

export function resolveProviderLogoAssetId(providerId: string): string | null {
  const normalized = providerId.trim().toLowerCase();
  if (!normalized) return null;

  const mapped = PROVIDER_LOGO_MAPPING[normalized];
  if (mapped) return mapped;

  if (PROVIDER_LOGO_ASSET_IDS.includes(normalized)) {
    return normalized;
  }

  return null;
}

export function providerLogoFallbackInitial(providerId: string): string {
  const trimmed = providerId.trim();
  if (!trimmed) return "?";
  return trimmed.slice(0, 1).toUpperCase();
}
