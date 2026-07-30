import { describe, expect, it } from "vitest";
import { mapForkUpdate } from "./updater";

describe("fork updater", () => {
  it("maps the newest fork prerelease returned by the backend", () => {
    expect(
      mapForkUpdate({
        currentVersion: "m3.18.0-1",
        availableVersion: "m3.18.0-2",
        notes: "fork changes",
        pubDate: "2026-07-26T17:07:21Z",
        releaseUrl:
          "https://github.com/qianbkk/cc-switch/releases/tag/m3.18.0-2",
      }),
    ).toEqual({
      status: "available",
      info: {
        currentVersion: "m3.18.0-1",
        availableVersion: "m3.18.0-2",
        notes: "fork changes",
        pubDate: "2026-07-26T17:07:21Z",
        releaseUrl:
          "https://github.com/qianbkk/cc-switch/releases/tag/m3.18.0-2",
      },
    });
  });

  it("normalizes nullable optional release fields", () => {
    expect(
      mapForkUpdate({
        currentVersion: "m3.18.0-1",
        availableVersion: "m3.18.0-2",
        notes: null,
        pubDate: null,
        releaseUrl:
          "https://github.com/qianbkk/cc-switch/releases/tag/m3.18.0-2",
      }),
    ).toEqual({
      status: "available",
      info: {
        currentVersion: "m3.18.0-1",
        availableVersion: "m3.18.0-2",
        notes: undefined,
        pubDate: undefined,
        releaseUrl:
          "https://github.com/qianbkk/cc-switch/releases/tag/m3.18.0-2",
      },
    });
  });

  it("reports up to date when the fork has no newer m release", () => {
    expect(mapForkUpdate(null)).toEqual({ status: "up-to-date" });
  });
});
