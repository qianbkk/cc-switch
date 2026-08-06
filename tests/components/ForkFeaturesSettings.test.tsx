import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ForkFeaturesSettings } from "@/components/settings/ForkFeaturesSettings";
import type { SettingsFormState } from "@/hooks/useSettings";

const openForkChangesHtmlMock = vi.fn();

vi.mock("@/lib/api", () => ({
  settingsApi: {
    openForkChangesHtml: (...args: unknown[]) =>
      openForkChangesHtmlMock(...args),
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function renderSettings(enabled: boolean, onChange = vi.fn()) {
  render(
    <ForkFeaturesSettings
      settings={{ forkFeaturesEnabled: enabled } as SettingsFormState}
      onChange={onChange}
    />,
  );
  return onChange;
}

describe("ForkFeaturesSettings", () => {
  it("uses the core runtime label and emits toggle changes", () => {
    const onChange = renderSettings(true);

    const toggle = screen.getByRole("switch", {
      name: "settings.forkFeaturesEnabled",
    });
    expect(toggle).toBeChecked();

    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith({ forkFeaturesEnabled: false });
  });

  it("shows the precise disabled hint without hiding fork details", () => {
    renderSettings(false);

    expect(
      screen.getByText("settings.forkFeaturesDisabledHint"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /settings\.forkChanges/ }),
    ).toBeInTheDocument();
  });
});
