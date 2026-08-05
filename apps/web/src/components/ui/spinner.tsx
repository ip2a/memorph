import { cn } from "@/lib/utils"
import { Loader2Icon } from "lucide-react"
import { useI18n } from "@/lib/i18n-context"

function Spinner({ className, ...props }: React.ComponentProps<"svg">) {
  const { t } = useI18n()
  return (
    <Loader2Icon data-slot="spinner" role="status" aria-label={t("loading")} className={cn("size-4 animate-spin", className)} {...props} />
  )
}

export { Spinner }
