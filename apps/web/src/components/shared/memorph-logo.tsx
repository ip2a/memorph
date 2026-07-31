import memorphLogo from "@/assets/memorph-logo.png";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { cn } from "@/lib/utils";

const sizeClasses = {
  sm: "size-8",
  md: "size-12",
  lg: "size-16",
} as const;

type MemorphLogoSize = keyof typeof sizeClasses;

type MemorphLogoProps = {
  size?: MemorphLogoSize;
  className?: string;
};

export function MemorphLogo({ size = "md", className }: MemorphLogoProps) {
  return (
    <Avatar className={cn(sizeClasses[size], "rounded-xl", className)} data-memorph-logo>
      <AvatarImage src={memorphLogo} alt="memorph" className="rounded-xl object-cover" />
      <AvatarFallback className="rounded-xl font-mono text-xs font-bold">M</AvatarFallback>
    </Avatar>
  );
}
