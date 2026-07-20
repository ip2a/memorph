import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  providerLogoFallbackInitial,
  resolveProviderLogoAssetId,
} from "@/components/shared/provider-logo-map";
import { cn } from "@/lib/utils";

const providerLogoModules = import.meta.glob<string>(
  "../../assets/provider-logos/*.svg",
  { eager: true, query: "?url", import: "default" },
);

const providerLogoUrls = Object.fromEntries(
  Object.entries(providerLogoModules).flatMap(([path, url]) => {
    const match = path.match(/\/([^/]+)\.svg$/);
    return match ? [[match[1], url]] : [];
  }),
) as Record<string, string>;

const syntheticLogoUrl = providerLogoUrls.synthetic;

export type ProviderLogoSize = "xs" | "sm" | "md";

const sizeConfig: Record<
  ProviderLogoSize,
  { avatarSize: "sm" | "default" | "lg"; className: string; imageClassName: string }
> = {
  xs: {
    avatarSize: "sm",
    className: "size-3.5 data-[size=sm]:size-3.5",
    imageClassName: "rounded-[3px] text-[10px]",
  },
  sm: {
    avatarSize: "default",
    className: "size-7 data-[size=default]:size-7",
    imageClassName: "rounded-md text-xs",
  },
  md: {
    avatarSize: "lg",
    className: "size-9 data-[size=lg]:size-9",
    imageClassName: "rounded-md text-sm",
  },
};

function resolveProviderLogoUrl(providerId: string): string | undefined {
  const assetId = resolveProviderLogoAssetId(providerId);
  if (assetId && providerLogoUrls[assetId]) {
    return providerLogoUrls[assetId];
  }
  return syntheticLogoUrl;
}

type ProviderLogoProps = {
  providerId: string;
  size?: ProviderLogoSize;
  className?: string;
  alt?: string;
};

export function ProviderLogo({
  providerId,
  size = "sm",
  className,
  alt,
}: ProviderLogoProps) {
  const config = sizeConfig[size];
  const src = resolveProviderLogoUrl(providerId);
  const label = alt ?? providerId;

  return (
    <Avatar
      size={config.avatarSize}
      className={cn(
        "rounded-md bg-transparent after:hidden dark:after:hidden",
        config.className,
        className,
      )}
      aria-hidden={alt === undefined ? true : undefined}
      title={label}
    >
      {src ? (
        <AvatarImage
          src={src}
          alt={label}
          className={cn("object-contain", config.imageClassName)}
        />
      ) : null}
      <AvatarFallback className={cn("rounded-md bg-transparent font-medium", config.imageClassName)}>
        {providerLogoFallbackInitial(providerId)}
      </AvatarFallback>
    </Avatar>
  );
}
