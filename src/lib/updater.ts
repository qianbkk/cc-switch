import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

export type UpdateChannel = "stable" | "beta";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
  releaseUrl: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

export interface ForkUpdatePayload {
  currentVersion: string;
  availableVersion: string;
  notes?: string | null;
  pubDate?: string | null;
  releaseUrl: string;
}

export const FORK_RELEASES_URL =
  "https://github.com/qianbkk/cc-switch/releases";

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export function mapForkUpdate(
  update: ForkUpdatePayload | null,
): { status: "up-to-date" } | { status: "available"; info: UpdateInfo } {
  if (!update) {
    return { status: "up-to-date" };
  }

  return {
    status: "available",
    info: {
      currentVersion: update.currentVersion,
      availableVersion: update.availableVersion,
      notes: update.notes ?? undefined,
      pubDate: update.pubDate ?? undefined,
      releaseUrl: update.releaseUrl,
    },
  };
}

export async function checkForUpdate(
  _opts: CheckOptions = {},
): Promise<
  { status: "up-to-date" } | { status: "available"; info: UpdateInfo }
> {
  // 魔改版只跟随 qianbkk/cc-switch 的 m* 预发行版。GitHub 的
  // releases/latest 不包含 prerelease，因此由后端调用 Releases API，按
  // m<upstream-version>-<revision> 规则比较，避免再次落回上游 latest.json。
  const update = await invoke<ForkUpdatePayload | null>("check_fork_update");

  return mapForkUpdate(update);
}

export async function openForkRelease(releaseUrl?: string): Promise<void> {
  await invoke("open_fork_release", { releaseUrl });
}
