import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@/components/Sidebar", () => ({
  default: () => <aside data-testid="sidebar" />,
}));

vi.mock("@/views/ScanView", () => ({ default: () => <div /> }));
vi.mock("@/views/CacheView", () => ({ default: () => <div /> }));
vi.mock("@/views/HistoryView", () => ({ default: () => <div /> }));
vi.mock("@/views/SettingsView", () => ({ default: () => <div /> }));
vi.mock("@/views/ProcessView", () => ({ default: () => <div /> }));
vi.mock("@/views/ApplicationsView", () => ({ default: () => <div /> }));
vi.mock("@/views/UninstallerView", () => ({ default: () => <div /> }));

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

import App from "@/App";

describe("App event lifecycle", () => {
  beforeEach(() => {
    mocks.listen.mockReset();
    localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("unlistens the tray listener on cleanup", async () => {
    const unlisten = vi.fn().mockResolvedValue(undefined);
    mocks.listen.mockResolvedValue(unlisten);
    const view = render(() => <App />);

    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(1));
    view.unmount();

    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(1));
  });

  it("catches a rejected tray unlisten promise", async () => {
    const unlisten = vi.fn().mockRejectedValue(new Error("unlisten failed"));
    mocks.listen.mockResolvedValue(unlisten);
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const view = render(() => <App />);

    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(1));
    view.unmount();

    await waitFor(() => expect(errorSpy).toHaveBeenCalled());
  });
});
