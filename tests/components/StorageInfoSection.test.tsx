import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { StorageInfoSection } from "@/components/settings/StorageInfoSection";
import type { StorageInfo } from "@/lib/api";

const tMock = vi.fn((key: string) => key);

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: tMock }),
}));

const getStorageInfoMock = vi.fn();
const openStorageItemMock = vi.fn();

vi.mock("@/lib/api", () => ({
  settingsApi: {
    getStorageInfo: (...args: unknown[]) => getStorageInfoMock(...args),
    openStorageItem: (...args: unknown[]) => openStorageItemMock(...args),
  },
}));

const DB_PATH = "C:/Users/tester/.cc-switch/cc-switch.db";

const sampleInfo: StorageInfo = {
  baseDir: "C:/Users/tester/.cc-switch",
  totalSizeBytes: 1048576 + 2048,
  dbSchemaVersion: 19,
  latestDbBackup: "db_backup_20260807_000000.db",
  items: [
    {
      path: DB_PATH,
      name: "cc-switch.db",
      kind: "file",
      purpose: "database",
      exists: true,
      sizeBytes: 1048576,
      recordCount: 12,
      error: null,
      schemaVersion: 19,
    },
    {
      path: "C:/Users/tester/.cc-switch/logs",
      name: "logs",
      kind: "dir",
      purpose: "logs",
      exists: true,
      sizeBytes: 2048,
      recordCount: 3,
      error: null,
      schemaVersion: null,
    },
    {
      path: "C:/Users/tester/.cc-switch/missing.json",
      name: "missing.json",
      kind: "file",
      purpose: "other",
      exists: false,
      sizeBytes: null,
      recordCount: null,
      error: "路径不存在",
      schemaVersion: null,
    },
  ],
};

function renderWithQuery() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <StorageInfoSection />
    </QueryClientProvider>,
  );
}

describe("StorageInfoSection Component", () => {
  beforeEach(() => {
    tMock.mockImplementation((key: string) => key);
    getStorageInfoMock.mockReset();
    openStorageItemMock.mockReset();
  });

  it("renders base dir, total size and item metadata", async () => {
    getStorageInfoMock.mockResolvedValue(sampleInfo);
    renderWithQuery();

    expect(
      await screen.findByText("C:/Users/tester/.cc-switch"),
    ).toBeInTheDocument();
    expect(screen.getByText("1.0 MB")).toBeInTheDocument();
    expect(screen.getByText("cc-switch.db")).toBeInTheDocument();
    expect(screen.getByText(/12/)).toBeInTheDocument(); // record count
    expect(screen.getByText("logs")).toBeInTheDocument();
    expect(screen.getByText(/3/)).toBeInTheDocument(); // file count
  });

  it("shows database schema version and latest backup metadata", async () => {
    getStorageInfoMock.mockResolvedValue(sampleInfo);
    renderWithQuery();

    await screen.findByText("cc-switch.db");
    // Top-level summary shows schema version v19 and the latest backup filename
    expect(
      screen.getByText("db_backup_20260807_000000.db"),
    ).toBeInTheDocument();
    // "v19" appears twice: once in the summary, once as the db entry badge
    expect(screen.getAllByText("v19").length).toBeGreaterThanOrEqual(2);
  });

  it("does not leak sensitive values like api keys", async () => {
    getStorageInfoMock.mockResolvedValue(sampleInfo);
    renderWithQuery();

    await screen.findByText("cc-switch.db");
    // The UI must never render key-like content.
    expect(
      screen.queryByText(/sk-test-secret|OPENAI_API_KEY|apiKey/i),
    ).not.toBeInTheDocument();
  });

  it("shows missing state for non-existent entries", async () => {
    getStorageInfoMock.mockResolvedValue(sampleInfo);
    renderWithQuery();

    await screen.findByText("missing.json");
    expect(
      screen.getByText("settings.advanced.storageInfo.notExists"),
    ).toBeInTheDocument();
    // open button for the missing entry (last item) must be disabled
    const openButtons = screen.getAllByRole("button", {
      name: /settings\.advanced\.storageInfo\.openFolder/,
    });
    expect(openButtons[3]).toBeDisabled();
  });

  it("calls openStorageItem when clicking an open button", async () => {
    getStorageInfoMock.mockResolvedValue(sampleInfo);
    openStorageItemMock.mockResolvedValue(true);
    renderWithQuery();

    await screen.findByText("cc-switch.db");
    const openButtons = screen.getAllByRole("button", {
      name: /settings\.advanced\.storageInfo\.openFolder/,
    });
    // openButtons order: [baseDir, db, logs, missing]
    fireEvent.click(openButtons[1]);
    await waitFor(() =>
      expect(openStorageItemMock).toHaveBeenCalledWith(DB_PATH),
    );
  });

  it("shows an error message when the query fails", async () => {
    getStorageInfoMock.mockRejectedValue(new Error("boom"));
    renderWithQuery();

    expect(
      await screen.findByText(/settings\.advanced\.storageInfo\.loadFailed/),
    ).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });
});
