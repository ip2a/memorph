import { FolderOpenIcon } from "lucide-react";
import { useState } from "react";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { WorkspaceSwitchDialog } from "@/features/workspaces/workspace-switch-dialog";
import { useI18n } from "@/lib/i18n-context";

export function WorkspacePathPicker({
  id,
  value,
  onChange,
  placeholder,
}: {
  id: string;
  value: string;
  onChange: (path: string) => void;
  placeholder?: string;
}) {
  const { t } = useI18n();
  const [pickerOpen, setPickerOpen] = useState(false);

  function openPicker() {
    setPickerOpen(true);
  }

  return (
    <>
      <InputGroup>
        <InputGroupInput
          id={id}
          readOnly
          value={value}
          placeholder={placeholder}
          className="cursor-default"
          onClick={openPicker}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              openPicker();
            }
          }}
        />
        <InputGroupAddon align="inline-end">
          <InputGroupButton type="button" variant="ghost" onClick={openPicker}>
            <FolderOpenIcon data-icon="inline-start" />
            {t("sessionBrowse")}
          </InputGroupButton>
        </InputGroupAddon>
      </InputGroup>
      <WorkspaceSwitchDialog
        mode="pick"
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        value={value}
        onPick={onChange}
      />
    </>
  );
}
