import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import SafeDragSurface from "@/components/shell/SafeDragSurface";

describe("SafeDragSurface", () => {
  beforeEach(() => tauriMocks.startDragging.mockClear());
  afterEach(cleanup);

  it("directTargetOnly drags only the surface itself", () => {
    render(() => (
      <SafeDragSurface directTargetOnly data-testid="surface">
        <span data-testid="text">可选择文本</span>
        <div data-testid="scroll" class="overflow-auto">滚动内容</div>
        <button onClick={() => undefined}>操作</button>
      </SafeDragSurface>
    ));

    const surface = screen.getByTestId("surface");
    const text = screen.getByTestId("text");
    const scroll = screen.getByTestId("scroll");
    const button = screen.getByRole("button", { name: "操作" });

    fireEvent.mouseDown(surface, { button: 0 });
    const textEvent = new MouseEvent("mousedown", {
      bubbles: true,
      cancelable: true,
      button: 0,
    });
    text.dispatchEvent(textEvent);
    fireEvent.mouseDown(scroll, { button: 0 });
    fireEvent.mouseDown(button, { button: 0 });
    fireEvent.click(button);

    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
    expect(textEvent.defaultPrevented).toBe(false);
  });

  it("drags from the surface but preserves nested controls", () => {
    let clicks = 0;
    render(() => (
      <SafeDragSurface data-testid="surface">
        <button onClick={() => clicks += 1}>操作</button>
      </SafeDragSurface>
    ));

    const surface = screen.getByTestId("surface");
    const button = screen.getByRole("button", { name: "操作" });

    fireEvent.mouseDown(surface, { button: 0 });
    fireEvent.mouseDown(button, { button: 0 });
    fireEvent.click(button);

    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
    expect(clicks).toBe(1);
  });
});
