import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Loader2, FileText, Copy } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { claudePluginApi, type ClaudePluginStatus } from "@/lib/api";
import { copyText } from "@/lib/clipboard";

export function ClaudePluginStatusPanel() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ClaudePluginStatus | null>(null);
  const [applied, setApplied] = useState<boolean>(false);
  const [isLoading, setIsLoading] = useState(true);
  const [showConfig, setShowConfig] = useState(false);
  const [configText, setConfigText] = useState<string | null>(null);
  const [isReading, setIsReading] = useState(false);

  useEffect(() => {
    Promise.all([claudePluginApi.getStatus(), claudePluginApi.isApplied()])
      .then(([s, a]) => {
        setStatus(s);
        setApplied(a);
      })
      .catch((e) => {
        console.error("Failed to load claude plugin status:", e);
        toast.error(String(e));
      })
      .finally(() => setIsLoading(false));
  }, []);

  const handleViewConfig = async () => {
    setShowConfig(true);
    setIsReading(true);
    try {
      const text = await claudePluginApi.readConfig();
      setConfigText(text);
    } catch (e) {
      console.error("Failed to read claude plugin config:", e);
      toast.error(String(e));
      setConfigText(null);
    } finally {
      setIsReading(false);
    }
  };

  const prettyConfig = (() => {
    if (configText == null) return null;
    try {
      return JSON.stringify(JSON.parse(configText), null, 2);
    } catch {
      return configText;
    }
  })();

  if (isLoading) {
    return (
      <div className="flex justify-center py-6">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <Label className="text-xs text-muted-foreground">
          {t("settings.advanced.claudePlugin.configPath")}
        </Label>
        <code className="block rounded bg-background px-2 py-1.5 text-xs font-mono break-all">
          {status?.path ?? "—"}
        </code>
      </div>

      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label>{t("settings.advanced.claudePlugin.skipFlag")}</Label>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.claudePlugin.skipFlagDescription")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant={applied ? "default" : "secondary"}>
            {applied
              ? t("settings.advanced.claudePlugin.applied")
              : t("settings.advanced.claudePlugin.notApplied")}
          </Badge>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void handleViewConfig()}
            disabled={!status?.exists}
          >
            <FileText className="mr-2 h-3.5 w-3.5" />
            {t("settings.advanced.claudePlugin.viewConfig")}
          </Button>
        </div>
      </div>

      <Dialog open={showConfig} onOpenChange={setShowConfig}>
        <DialogContent className="max-w-2xl glass border-border">
          <DialogHeader>
            <DialogTitle className="flex items-center justify-between gap-2 pr-8">
              <span>{t("settings.advanced.claudePlugin.viewConfigTitle")}</span>
              {prettyConfig != null && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7"
                  onClick={() => {
                    void copyText(prettyConfig)
                      .then(() =>
                        toast.success(
                          t("common.copied", { defaultValue: "已复制" }),
                        ),
                      )
                      .catch((e) => {
                        console.error("Failed to copy config:", e);
                        toast.error(String(e));
                      });
                  }}
                >
                  <Copy className="mr-1.5 h-3 w-3" />
                  {t("common.copy", { defaultValue: "复制" })}
                </Button>
              )}
            </DialogTitle>
          </DialogHeader>
          <div className="px-6 pb-4">
            {isReading ? (
              <div className="flex justify-center py-6">
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
              </div>
            ) : prettyConfig == null ? (
              <p className="text-sm text-muted-foreground">
                {t("settings.advanced.claudePlugin.noConfig")}
              </p>
            ) : (
              <pre className="max-h-[50vh] overflow-auto rounded-lg bg-muted/50 p-3 text-xs font-mono whitespace-pre">
                {prettyConfig}
              </pre>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
