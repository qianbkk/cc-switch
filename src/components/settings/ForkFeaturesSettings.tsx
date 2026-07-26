import { useTranslation } from "react-i18next";
import { FlaskConical, GitCompare } from "lucide-react";
import type { SettingsFormState } from "@/hooks/useSettings";
import { ToggleRow } from "@/components/ui/toggle-row";
import { Button } from "@/components/ui/button";
import { settingsApi } from "@/lib/api";
import { toast } from "sonner";

interface ForkFeaturesSettingsProps {
  settings: SettingsFormState;
  onChange: (
    updates: Partial<SettingsFormState>,
  ) => void | boolean | Promise<void | boolean>;
}

/**
 * 魔改功能总开关（fork 专属）。
 *
 * 关闭后所有 fork 独有行为一律停用，应用回到与上游原版一致的行为：
 * 统一网关拒绝服务、Live 配置保护不拦截、Codex auth 反向同步不执行、
 * 魔改新增的设置面板全部隐藏、被隐藏的上游入口重新出现。
 *
 * 配置数据全部保留在数据库里，重新打开即恢复原样。
 */
export function ForkFeaturesSettings({
  settings,
  onChange,
}: ForkFeaturesSettingsProps) {
  const { t } = useTranslation();
  const enabled = settings.forkFeaturesEnabled ?? true;

  const handleOpenForkChanges = async () => {
    try {
      await settingsApi.openForkChangesHtml();
    } catch (error) {
      console.error("[ForkFeaturesSettings] open fork changes failed", error);
      toast.error(t("settings.openForkChangesFailed"));
    }
  };

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between gap-2 pb-2 border-b border-border/40">
        <div className="flex items-center gap-2">
          <FlaskConical className="h-4 w-4 text-primary" />
          <h3 className="text-sm font-medium">{t("settings.forkFeatures")}</h3>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={handleOpenForkChanges}
          className="h-7 gap-1.5 text-xs"
        >
          <GitCompare className="h-3.5 w-3.5" />
          {t("settings.forkChanges")}
        </Button>
      </div>

      <ToggleRow
        icon={<FlaskConical className="h-4 w-4 text-violet-500" />}
        title={t("settings.forkFeaturesEnabled")}
        description={t("settings.forkFeaturesEnabledDescription")}
        checked={enabled}
        onCheckedChange={(value) => onChange({ forkFeaturesEnabled: value })}
      />

      {!enabled && (
        <p className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
          {t("settings.forkFeaturesDisabledHint")}
        </p>
      )}
    </section>
  );
}
