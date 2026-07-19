import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Minimize2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { lightweightApi } from "@/lib/api";

export function LightweightModeSettings() {
  const { t } = useTranslation();
  const [showConfirm, setShowConfirm] = useState(false);

  const handleEnter = async () => {
    setShowConfirm(false);
    try {
      await lightweightApi.enter();
    } catch (e) {
      console.error("Failed to enter lightweight mode:", e);
      toast.error(String(e));
    }
  };

  return (
    <div className="rounded-xl glass-card p-6">
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <Minimize2 className="h-5 w-5 text-indigo-500" />
          <div className="space-y-0.5">
            <Label className="text-base font-semibold">
              {t("settings.lightweight.title")}
            </Label>
            <p className="text-sm text-muted-foreground font-normal">
              {t("settings.lightweight.description")}
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          className="shrink-0"
          onClick={() => setShowConfirm(true)}
        >
          {t("settings.lightweight.enter")}
        </Button>
      </div>

      <ConfirmDialog
        isOpen={showConfirm}
        variant="info"
        title={t("settings.lightweight.confirmTitle")}
        message={t("settings.lightweight.confirmMessage")}
        confirmText={t("settings.lightweight.confirmButton")}
        onConfirm={() => void handleEnter()}
        onCancel={() => setShowConfirm(false)}
      />
    </div>
  );
}
