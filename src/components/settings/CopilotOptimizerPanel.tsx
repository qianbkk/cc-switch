import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Loader2 } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
  copilotOptimizerApi,
  type CopilotOptimizerConfig,
} from "@/lib/api";

/** 布尔开关字段（warmupModel 单独处理） */
const TOGGLE_FIELDS: Array<keyof CopilotOptimizerConfig> = [
  "enabled",
  "requestClassification",
  "toolResultMerging",
  "compactDetection",
  "deterministicRequestId",
  "subagentDetection",
  "warmupDowngrade",
  "stripThinking",
];

export function CopilotOptimizerPanel() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<CopilotOptimizerConfig | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  // 保存最后一次"已确认持久化"的值，便于失败回滚（包括 warmupModel 本地输入）。
  const lastSavedRef = useRef<CopilotOptimizerConfig | null>(null);

  useEffect(() => {
    copilotOptimizerApi
      .getConfig()
      .then((c) => {
        setConfig(c);
        lastSavedRef.current = c;
      })
      .catch((e) => {
        console.error("Failed to load copilot optimizer config:", e);
        toast.error(String(e));
      })
      .finally(() => setIsLoading(false));
  }, []);

  const handleChange = async (updates: Partial<CopilotOptimizerConfig>) => {
    if (!config) return;
    const prev = lastSavedRef.current ?? config;
    const next = { ...config, ...updates };
    setConfig(next);
    try {
      await copilotOptimizerApi.setConfig(next);
      lastSavedRef.current = next;
    } catch (e) {
      console.error("Failed to save copilot optimizer config:", e);
      toast.error(String(e));
      setConfig(prev);
    }
  };

  if (isLoading) {
    return (
      <div className="flex justify-center py-6">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!config) return null;

  const enabled = config.enabled;

  return (
    <div className="space-y-5">
      {TOGGLE_FIELDS.map((field) => {
        const isMaster = field === "enabled";
        return (
          <div key={field} className="flex items-center justify-between gap-4">
            <div className="space-y-0.5">
              <Label>
                {t(`settings.advanced.copilotOptimizer.fields.${field}.label`)}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t(
                  `settings.advanced.copilotOptimizer.fields.${field}.description`,
                )}
              </p>
            </div>
            <Switch
              checked={config[field] as boolean}
              disabled={!isMaster && !enabled}
              onCheckedChange={(checked) =>
                void handleChange({ [field]: checked })
              }
            />
          </div>
        );
      })}

      {/* Warmup 降级模型名 */}
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label>
            {t("settings.advanced.copilotOptimizer.fields.warmupModel.label")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.advanced.copilotOptimizer.fields.warmupModel.description",
            )}
          </p>
        </div>
        <Input
          value={config.warmupModel}
          disabled={!enabled || !config.warmupDowngrade}
          onChange={(e) => setConfig({ ...config, warmupModel: e.target.value })}
          onBlur={() => {
            const committed = lastSavedRef.current?.warmupModel;
            if (committed !== config.warmupModel) {
              void handleChange({ warmupModel: config.warmupModel });
            }
          }}
          className="h-8 w-40 text-xs font-mono"
        />
      </div>
    </div>
  );
}
