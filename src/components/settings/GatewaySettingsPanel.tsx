import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Copy,
  Eye,
  EyeOff,
  Loader2,
  RefreshCw,
  Plus,
  Trash2,
  Download,
  ChevronRight,
} from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  gatewayApi,
  buildGatewayAlias,
  providersApi,
  proxyApi,
  type GatewayConfig,
  type GatewayModelEntry,
  type AppId,
} from "@/lib/api";
import type { Provider } from "@/types";
import {
  fetchModelsForConfig,
  showFetchModelsError,
} from "@/lib/api/model-fetch";
import {
  getApiKeyFromConfig,
  getCodexBaseUrl,
} from "@/utils/providerConfigUtils";
import { copyText } from "@/lib/clipboard";

const GATEWAY_APPS = ["claude", "codex", "gemini"] as const;
type GatewayApp = (typeof GATEWAY_APPS)[number];

const APP_LABELS: Record<GatewayApp, string> = {
  claude: "Claude",
  codex: "Codex",
  gemini: "Gemini",
};

/** 从供应商配置中提取 Base URL（用于拉取模型） */
function getProviderBaseUrl(provider: Provider, appType: GatewayApp): string {
  if (appType === "codex") {
    return getCodexBaseUrl(provider) ?? "";
  }
  const env = provider.settingsConfig?.env as
    | Record<string, unknown>
    | undefined;
  if (appType === "gemini") {
    const v =
      (env?.GOOGLE_GEMINI_BASE_URL as string | undefined) ??
      (env?.GEMINI_BASE_URL as string | undefined);
    return typeof v === "string" ? v : "";
  }
  const v = env?.ANTHROPIC_BASE_URL;
  return typeof v === "string" ? v : "";
}

/** 从供应商配置中提取 API Key（用于拉取模型） */
function getProviderApiKey(provider: Provider, appType: GatewayApp): string {
  const raw =
    appType === "codex"
      ? typeof provider.settingsConfig?.config === "string"
        ? provider.settingsConfig.config
        : ""
      : JSON.stringify(provider.settingsConfig ?? {});
  // Codex 的 key 常在 auth（JSON）里而非 config（TOML），优先尝试 auth
  if (appType === "codex") {
    const auth =
      typeof provider.settingsConfig?.auth === "string"
        ? provider.settingsConfig.auth
        : JSON.stringify(provider.settingsConfig?.auth ?? {});
    const fromAuth = getApiKeyFromConfig(auth, "codex");
    if (fromAuth) return fromAuth;
  }
  return getApiKeyFromConfig(raw, appType);
}

interface FetchState {
  loading: boolean;
  models: string[];
  manualInput: string;
}

export function GatewaySettingsPanel() {
  const { t } = useTranslation();

  const [config, setConfig] = useState<GatewayConfig | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isToggling, setIsToggling] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [showRegenConfirm, setShowRegenConfirm] = useState(false);
  const [proxyPort, setProxyPort] = useState<number | null>(null);

  const [providersByApp, setProvidersByApp] = useState<
    Record<GatewayApp, Provider[]>
  >({ claude: [], codex: [], gemini: [] });
  const [fetchStates, setFetchStates] = useState<Record<string, FetchState>>(
    {},
  );

  // 加载网关配置 + 代理端口 + 各应用供应商
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cfg, providerLists, globalProxy] = await Promise.all([
          gatewayApi.getConfig(),
          Promise.all(
            GATEWAY_APPS.map((app) =>
              providersApi.getAll(app as AppId).catch(() => ({}) as Record<string, Provider>),
            ),
          ),
          proxyApi.getGlobalProxyConfig().catch(() => null),
        ]);
        if (cancelled) return;
        setConfig(cfg);
        setProxyPort(globalProxy?.listenPort ?? null);
        const grouped = {} as Record<GatewayApp, Provider[]>;
        GATEWAY_APPS.forEach((app, i) => {
          grouped[app] = Object.values(providerLists[i]).sort(
            (a, b) => (a.sortIndex ?? 0) - (b.sortIndex ?? 0),
          );
        });
        setProvidersByApp(grouped);
      } catch (e) {
        console.error("Failed to load gateway config:", e);
        if (!cancelled) toast.error(String(e));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const baseUrl = useMemo(() => {
    if (proxyPort == null || proxyPort <= 0) return null;
    return `http://127.0.0.1:${proxyPort}/gateway`;
  }, [proxyPort]);

  const providerNameById = useMemo(() => {
    const map = new Map<string, string>();
    GATEWAY_APPS.forEach((app) => {
      providersByApp[app].forEach((p) => map.set(p.id, p.name));
    });
    return map;
  }, [providersByApp]);

  const handleCopy = useCallback(
    async (text: string) => {
      try {
        await copyText(text);
        toast.success(t("common.copied", { defaultValue: "已复制" }));
      } catch (e) {
        toast.error(String(e));
      }
    },
    [t],
  );

  const persist = useCallback(
    async (next: GatewayConfig) => {
      setIsSaving(true);
      try {
        await gatewayApi.saveConfig(next);
        setConfig(next);
        return true;
      } catch (e) {
        console.error("Failed to save gateway config:", e);
        toast.error(String(e));
        return false;
      } finally {
        setIsSaving(false);
      }
    },
    [],
  );

  const handleToggleEnabled = useCallback(
    async (enabled: boolean) => {
      if (!config) return;
      setIsToggling(true);
      const next = { ...config, enabled };
      const ok = await persist(next);
      setIsToggling(false);
      if (ok) {
        toast.success(
          enabled
            ? t("settings.advanced.gateway.enabledToast", {
                defaultValue: "统一网关已启用",
              })
            : t("settings.advanced.gateway.disabledToast", {
                defaultValue: "统一网关已禁用",
              }),
        );
      }
    },
    [config, persist, t],
  );

  const handleRegenerate = useCallback(async () => {
    setShowRegenConfirm(false);
    if (!config) return;
    try {
      const newKey = await gatewayApi.regenerateKey();
      setConfig({ ...config, apiKey: newKey });
      setShowKey(true);
      toast.success(
        t("settings.advanced.gateway.keyRegenerated", {
          defaultValue: "已生成新的 API Key",
        }),
      );
    } catch (e) {
      console.error("Failed to regenerate gateway key:", e);
      toast.error(String(e));
    }
  }, [config, t]);

  const getFetchState = (key: string): FetchState =>
    fetchStates[key] ?? { loading: false, models: [], manualInput: "" };

  const setFetchState = (key: string, updates: Partial<FetchState>) => {
    setFetchStates((prev) => ({
      ...prev,
      [key]: { ...getFetchState(key), ...updates },
    }));
  };

  const handleFetchModels = useCallback(
    async (app: GatewayApp, provider: Provider) => {
      const key = `${app}:${provider.id}`;
      const baseUrlValue = getProviderBaseUrl(provider, app);
      const apiKeyValue = getProviderApiKey(provider, app);
      setFetchState(key, { loading: true });
      try {
        const models = await fetchModelsForConfig(baseUrlValue, apiKeyValue);
        setFetchState(key, {
          loading: false,
          models: models.map((m) => m.id),
        });
        if (models.length === 0) {
          toast.info(
            t("settings.advanced.gateway.noModelsFound", {
              defaultValue: "未获取到模型",
            }),
          );
        }
      } catch (e) {
        setFetchState(key, { loading: false });
        showFetchModelsError(e, t, {
          hasApiKey: Boolean(apiKeyValue),
          hasBaseUrl: Boolean(baseUrlValue),
        });
      }
    },
    [t],
  );

  const isModelSelected = useCallback(
    (app: GatewayApp, providerId: string, model: string): boolean =>
      Boolean(
        config?.models.some(
          (m) =>
            m.appType === app &&
            m.providerId === providerId &&
            m.model === model,
        ),
      ),
    [config],
  );

  const toggleModel = useCallback(
    (app: GatewayApp, provider: Provider, model: string, checked: boolean) => {
      if (!config) return;
      const alias = buildGatewayAlias(provider.name, model);
      let nextModels: GatewayModelEntry[];
      if (checked) {
        if (isModelSelected(app, provider.id, model)) return;
        nextModels = [
          ...config.models,
          { alias, providerId: provider.id, appType: app, model },
        ];
      } else {
        nextModels = config.models.filter(
          (m) =>
            !(
              m.appType === app &&
              m.providerId === provider.id &&
              m.model === model
            ),
        );
      }
      setConfig({ ...config, models: nextModels });
    },
    [config, isModelSelected],
  );

  const addManualModel = useCallback(
    (app: GatewayApp, provider: Provider) => {
      if (!config) return;
      const key = `${app}:${provider.id}`;
      const model = getFetchState(key).manualInput.trim();
      if (!model) return;
      if (isModelSelected(app, provider.id, model)) {
        setFetchState(key, { manualInput: "" });
        return;
      }
      const alias = buildGatewayAlias(provider.name, model);
      setConfig({
        ...config,
        models: [
          ...config.models,
          { alias, providerId: provider.id, appType: app, model },
        ],
      });
      setFetchState(key, { manualInput: "" });
    },
    [config, isModelSelected],
  );

  const removeModel = useCallback(
    (entry: GatewayModelEntry) => {
      if (!config) return;
      setConfig({
        ...config,
        models: config.models.filter(
          (m) =>
            !(
              m.appType === entry.appType &&
              m.providerId === entry.providerId &&
              m.model === entry.model
            ),
        ),
      });
    },
    [config],
  );

  const handleSaveModels = useCallback(async () => {
    if (!config) return;
    const ok = await persist(config);
    if (ok) {
      toast.success(t("common.saved", { defaultValue: "已保存" }));
    }
  }, [config, persist, t]);

  // 模型池按供应商分组展示
  const groupedSelected = useMemo(() => {
    if (!config) return [] as Array<{ key: string; label: string; entries: GatewayModelEntry[] }>;
    const groups = new Map<string, GatewayModelEntry[]>();
    for (const m of config.models) {
      const gkey = `${m.appType}:${m.providerId}`;
      if (!groups.has(gkey)) groups.set(gkey, []);
      groups.get(gkey)!.push(m);
    }
    return Array.from(groups.entries()).map(([gkey, entries]) => {
      const [app, providerId] = gkey.split(":");
      const providerName = providerNameById.get(providerId) ?? providerId;
      return {
        key: gkey,
        label: `${APP_LABELS[app as GatewayApp]} · ${providerName}`,
        entries,
      };
    });
  }, [config, providerNameById]);

  const curlExample = useMemo(() => {
    const alias = config?.models[0]?.alias ?? "provider/model";
    const key = config?.apiKey || "YOUR_GATEWAY_KEY";
    const url = baseUrl ?? "http://127.0.0.1:<PORT>/gateway";
    return [
      `curl ${url}/v1/chat/completions \\`,
      `  -H "Authorization: Bearer ${key}" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{"model":"${alias}","messages":[{"role":"user","content":"Hello"}]}'`,
    ].join("\n");
  }, [baseUrl, config]);

  if (isLoading) {
    return (
      <div className="flex justify-center py-6">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!config) {
    return (
      <p className="text-sm text-muted-foreground">
        {t("settings.advanced.gateway.loadFailed", {
          defaultValue: "无法加载网关配置",
        })}
      </p>
    );
  }

  const maskedKey = config.apiKey
    ? `${config.apiKey.slice(0, 4)}${"•".repeat(Math.max(config.apiKey.length - 8, 4))}${config.apiKey.slice(-4)}`
    : "";

  return (
    <div className="space-y-6">
      {/* 启用开关 */}
      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <Label>{t("settings.advanced.gateway.enabled")}</Label>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.enabledDescription")}
          </p>
        </div>
        <Switch
          checked={config.enabled}
          disabled={isToggling}
          onCheckedChange={(checked) => void handleToggleEnabled(checked)}
        />
      </div>

      {/* 接入信息 */}
      <div className="rounded-lg bg-muted/50 p-4 space-y-3">
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.baseUrl")}
          </Label>
          <div className="flex items-center gap-2">
            <code className="flex-1 rounded bg-background px-2 py-1.5 text-xs font-mono break-all">
              {baseUrl ?? t("settings.advanced.gateway.baseUrlUnknown", {
                defaultValue: "代理端口未配置，请先在代理设置中启用并配置端口",
              })}
            </code>
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              disabled={!baseUrl}
              onClick={() => baseUrl && void handleCopy(baseUrl)}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.endpointsHint")}
          </p>
        </div>

        {/* API Key */}
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.apiKey")}
          </Label>
          <div className="flex items-center gap-2">
            <code className="flex-1 rounded bg-background px-2 py-1.5 text-xs font-mono break-all">
              {config.apiKey ? (showKey ? config.apiKey : maskedKey) : "—"}
            </code>
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              onClick={() => setShowKey((v) => !v)}
              disabled={!config.apiKey}
            >
              {showKey ? (
                <EyeOff className="h-3.5 w-3.5" />
              ) : (
                <Eye className="h-3.5 w-3.5" />
              )}
            </Button>
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              onClick={() => void handleCopy(config.apiKey)}
              disabled={!config.apiKey}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              onClick={() => setShowRegenConfirm(true)}
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>

      {/* 模型池管理 */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.gateway.modelPool")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.gateway.modelPoolDescription")}
            </p>
          </div>
          <Button size="sm" onClick={() => void handleSaveModels()} disabled={isSaving}>
            {isSaving ? (
              <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
            ) : null}
            {t("common.save")}
          </Button>
        </div>

        {GATEWAY_APPS.map((app) => {
          const providers = providersByApp[app];
          if (providers.length === 0) return null;
          return (
            <div key={app} className="space-y-2">
              <div className="flex items-center gap-2">
                <span className="text-xs font-semibold text-muted-foreground">
                  {APP_LABELS[app]}
                </span>
              </div>
              {providers.map((provider) => {
                const fkey = `${app}:${provider.id}`;
                const state = getFetchState(fkey);
                return (
                  <Collapsible
                    key={fkey}
                    className="rounded-lg border border-border/50"
                  >
                    <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-muted/50 [&[data-state=open]>svg]:rotate-90">
                      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform" />
                      <span className="flex-1 truncate">{provider.name}</span>
                    </CollapsibleTrigger>
                    <CollapsibleContent className="px-3 pb-3 pt-1 space-y-3 border-t border-border/50">
                      <div className="pt-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() =>
                            void handleFetchModels(app, provider)
                          }
                          disabled={state.loading}
                        >
                          {state.loading ? (
                            <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Download className="mr-2 h-3.5 w-3.5" />
                          )}
                          {t("settings.advanced.gateway.fetchModels")}
                        </Button>
                      </div>

                      {state.models.length > 0 && (
                        <div className="space-y-1.5 max-h-48 overflow-y-auto">
                          {state.models.map((model) => (
                            <label
                              key={model}
                              className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-xs hover:bg-muted/50"
                            >
                              <Checkbox
                                checked={isModelSelected(
                                  app,
                                  provider.id,
                                  model,
                                )}
                                onCheckedChange={(v) =>
                                  toggleModel(app, provider, model, v === true)
                                }
                              />
                              <span className="font-mono">{model}</span>
                            </label>
                          ))}
                        </div>
                      )}

                      {/* 手动添加 */}
                      <div className="flex items-center gap-2">
                        <Input
                          value={state.manualInput}
                          onChange={(e) =>
                            setFetchState(fkey, {
                              manualInput: e.target.value,
                            })
                          }
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              addManualModel(app, provider);
                            }
                          }}
                          placeholder={t(
                            "settings.advanced.gateway.manualModelPlaceholder",
                          )}
                          className="h-8 text-xs"
                        />
                        <Button
                          variant="outline"
                          size="icon"
                          className="h-8 w-8 shrink-0"
                          onClick={() => addManualModel(app, provider)}
                        >
                          <Plus className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </CollapsibleContent>
                  </Collapsible>
                );
              })}
            </div>
          );
        })}
      </div>

      {/* 已选模型列表（分组） */}
      {groupedSelected.length > 0 && (
        <div className="space-y-2">
          <Label className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.selectedModels")}
          </Label>
          {groupedSelected.map((group) => (
            <div key={group.key} className="space-y-1.5">
              <p className="text-xs font-semibold text-muted-foreground">
                {group.label}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {group.entries.map((entry) => (
                  <Badge
                    key={entry.alias}
                    variant="secondary"
                    className="gap-1.5 font-mono"
                  >
                    {entry.alias}
                    <button
                      type="button"
                      className="text-muted-foreground hover:text-destructive"
                      onClick={() => removeModel(entry)}
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </Badge>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* curl 示例 */}
      <div className="space-y-1.5">
        <div className="flex items-center justify-between">
          <Label className="text-xs text-muted-foreground">
            {t("settings.advanced.gateway.curlExample")}
          </Label>
          <Button
            variant="ghost"
            size="sm"
            className="h-7"
            onClick={() => void handleCopy(curlExample)}
          >
            <Copy className="mr-1.5 h-3 w-3" />
            {t("common.copy", { defaultValue: "复制" })}
          </Button>
        </div>
        <pre className="rounded-lg bg-muted/50 p-3 text-xs font-mono overflow-x-auto whitespace-pre">
          {curlExample}
        </pre>
      </div>

      <ConfirmDialog
        isOpen={showRegenConfirm}
        variant="destructive"
        title={t("settings.advanced.gateway.regenConfirmTitle")}
        message={t("settings.advanced.gateway.regenConfirmMessage")}
        confirmText={t("settings.advanced.gateway.regenConfirmButton")}
        onConfirm={() => void handleRegenerate()}
        onCancel={() => setShowRegenConfirm(false)}
      />
    </div>
  );
}
