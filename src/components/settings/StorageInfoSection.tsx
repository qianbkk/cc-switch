import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  FolderOpen,
  RefreshCw,
  ShieldCheck,
  Database,
  FileText,
  FolderCog,
  HardDrive,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { settingsApi, type StorageItem } from "@/lib/api";
import { extractErrorMessage } from "@/utils/errorUtils";

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function purposeIcon(purpose: StorageItem["purpose"]) {
  switch (purpose) {
    case "database":
      return <Database className="h-4 w-4 text-blue-500" />;
    case "backups":
      return <HardDrive className="h-4 w-4 text-amber-500" />;
    case "logs":
      return <FileText className="h-4 w-4 text-zinc-500" />;
    default:
      return <FolderCog className="h-4 w-4 text-violet-500" />;
  }
}

/** 数据存储信息：仅展示元数据（路径/用途/大小/记录数），不含任何敏感值 */
export function StorageInfoSection() {
  const { t } = useTranslation();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["storageInfo"],
    queryFn: () => settingsApi.getStorageInfo(),
  });

  const handleOpen = useCallback(
    async (item: StorageItem) => {
      try {
        await settingsApi.openStorageItem(item.path);
      } catch (err) {
        toast.error(
          t("settings.advanced.storageInfo.openFailed", {
            defaultValue: "打开路径失败",
          }) + `: ${extractErrorMessage(err)}`,
        );
      }
    },
    [t],
  );

  return (
    <div className="space-y-4">
      <div className="flex items-start gap-2 text-xs text-muted-foreground">
        <ShieldCheck className="h-4 w-4 mt-0.5 shrink-0 text-green-600" />
        <p>
          {t("settings.advanced.storageInfo.securityNote", {
            defaultValue:
              "此处仅展示存储元数据（路径、用途、大小、记录数），不会读取或显示任何 API Key、Token 等敏感内容。",
          })}
        </p>
      </div>

      {/* 总览：数据目录 + 总占用 */}
      <div className="rounded-lg border border-border/60 bg-muted/30 p-4 space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <div className="text-xs font-medium text-muted-foreground">
              {t("settings.advanced.storageInfo.baseDir", {
                defaultValue: "数据目录",
              })}
            </div>
            <div className="truncate font-mono text-xs">
              {isLoading ? (
                <div className="h-4 w-64 animate-pulse rounded bg-muted" />
              ) : (
                (data?.baseDir ?? "—")
              )}
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {data && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => settingsApi.openStorageItem(data.baseDir)}
              >
                <FolderOpen className="h-3.5 w-3.5 mr-1.5" />
                {t("settings.advanced.storageInfo.openFolder", {
                  defaultValue: "打开目录",
                })}
              </Button>
            )}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              onClick={() => refetch()}
              disabled={isFetching}
              title={t("settings.advanced.storageInfo.refresh", {
                defaultValue: "刷新",
              })}
            >
              <RefreshCw
                className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`}
              />
            </Button>
          </div>
        </div>
        <div className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">
            {t("settings.advanced.storageInfo.totalSize", {
              defaultValue: "总占用",
            })}
          </span>
          <span className="font-semibold">
            {isLoading ? (
              <div className="h-4 w-16 animate-pulse rounded bg-muted" />
            ) : (
              formatBytes(data?.totalSizeBytes)
            )}
          </span>
        </div>
      </div>

      {/* 条目列表 */}
      <div className="space-y-2">
        {isLoading ? (
          Array.from({ length: 5 }).map((_, i) => (
            <div
              key={i}
              className="h-12 w-full animate-pulse rounded-lg bg-muted"
            />
          ))
        ) : isError ? (
          <p className="text-sm text-destructive">
            {t("settings.advanced.storageInfo.loadFailed", {
              defaultValue: "获取存储信息失败",
            })}
            : {extractErrorMessage(error)}
          </p>
        ) : !data || data.items.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.advanced.storageInfo.empty", {
              defaultValue: "暂无数据",
            })}
          </p>
        ) : (
          data.items.map((item) => (
            <div
              key={item.path}
              className="flex items-center gap-3 rounded-lg border border-border/50 bg-card/50 px-3 py-2.5"
            >
              <span className="shrink-0">{purposeIcon(item.purpose)}</span>
              <div className="min-w-0 flex-1 space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium">
                    {item.name}
                  </span>
                  <Badge
                    variant="secondary"
                    className="text-[10px] px-1.5 py-0"
                  >
                    {t(
                      `settings.advanced.storageInfo.purpose.${item.purpose}`,
                      { defaultValue: item.purpose },
                    )}
                  </Badge>
                  {!item.exists && (
                    <Badge
                      variant="destructive"
                      className="text-[10px] px-1.5 py-0"
                    >
                      {t("settings.advanced.storageInfo.notExists", {
                        defaultValue: "不存在",
                      })}
                    </Badge>
                  )}
                </div>
                <div className="flex items-center gap-3 text-xs text-muted-foreground">
                  <span>
                    {t("settings.advanced.storageInfo.size", {
                      defaultValue: "大小",
                    })}
                    : {formatBytes(item.sizeBytes)}
                  </span>
                  <span>
                    {t("settings.advanced.storageInfo.records", {
                      defaultValue: "记录",
                    })}
                    :{" "}
                    {item.recordCount != null
                      ? item.recordCount.toLocaleString()
                      : "—"}
                  </span>
                  {item.error && (
                    <span className="truncate text-amber-600">
                      {item.error}
                    </span>
                  )}
                </div>
              </div>
              <Button
                type="button"
                variant="outline"
                size="icon"
                className="h-8 w-8 shrink-0"
                disabled={!item.exists}
                onClick={() => handleOpen(item)}
                title={t("settings.advanced.storageInfo.openFolder", {
                  defaultValue: "打开目录",
                })}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </Button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
