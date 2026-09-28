import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import AppShell from "@/components/shell/AppShell";

const dispatchMouseDown = (target: Element) => {
  const event = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    button: 0,
  });
  target.dispatchEvent(event);
  return event;
};

describe("AppShell", () => {
  beforeEach(() => tauriMocks.startDragging.mockClear());
  afterEach(cleanup);

  it("工具栏、细条、内容区背景都可拖；卡片/按钮/受保护区不拖", () => {
    let clicks = 0;
    render(() => (
      <AppShell
        sidebar={<aside data-testid="sidebar">侧栏</aside>}
        toolbar={
          // 必须与 AppShell.tsx 里的真实 toolbar 一致地带 pointer-events-none。
          // 少了它，mousedown 的 target 会落在 h1 文字上而不是拖动面 div 上，
          // 于是「文字让位给拖选」规则会把工具栏误判成不可拖 —— 这是个会骗过
          // 人的夹具差异，测出来的行为和真机不一致。
          <h1 data-testid="toolbar" class="pointer-events-none">智能扫描</h1>
        }
      >
        <div data-testid="content">
          <span data-testid="content-text">可选择文本</span>
          <div data-testid="content-scroll" class="overflow-auto">滚动内容</div>
          <div data-testid="content-card" class="card">
            <code data-testid="card-path">/Users/…/some/cache/path</code>
          </div>
          <button onClick={() => clicks += 1}>内容操作</button>
        </div>
      </AppShell>
    ));

    const toolbarLabel = screen.getByTestId("toolbar");
    // toolbar 的 h1 带 pointer-events-none，真机上 mousedown 的 target 是它**父级**
    // 那个拖动面 div。jsdom 不实现 pointer-events 的命中测试，直接往 h1 上 dispatch
    // 模拟不出这个落点，所以这里显式取父元素，让夹具与真机一致。
    const toolbar = toolbarLabel.parentElement as Element;
    const gutter = screen.getByTestId("content-drag-gutter");
    const contentWrapper = screen.getByTestId("content-wrapper");
    const text = screen.getByTestId("content-text");
    const scroll = screen.getByTestId("content-scroll");
    const card = screen.getByTestId("content-card");
    const cardPath = screen.getByTestId("card-path");
    const button = screen.getByRole("button", { name: "内容操作" });
    const toolbarEvent = dispatchMouseDown(toolbar);
    const gutterEvent = dispatchMouseDown(gutter);
    const wrapperEvent = dispatchMouseDown(contentWrapper);
    const textEvent = dispatchMouseDown(text);
    const scrollEvent = dispatchMouseDown(scroll);
    const cardEvent = dispatchMouseDown(card);
    const cardPathEvent = dispatchMouseDown(cardPath);
    const buttonEvent = dispatchMouseDown(button);
    fireEvent.click(button);

    // 三个拖动面：工具栏 48px + 细条 10px + 整个内容区
    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(3);
    expect(toolbarEvent.defaultPrevented).toBe(true);
    expect(gutterEvent.defaultPrevented).toBe(true);
    // 内容区背景可拖 —— 这条是回归门禁。P0 重构把它换成了没有 handler 的普通
    // div，拖动区缩到顶部 58px。当时的用例把「内容区不拖」当设计意图断言了下来，
    // 于是退化过了 CI。别再把它改回去。
    expect(wrapperEvent.defaultPrevented).toBe(true);
    // 压在文字上时让位给拖选：路径文本、纯文本行、滚动区里的文字都能选中，
    // 代价是这些点不拖窗口。滚轮滚动不受影响。
    expect(textEvent.defaultPrevented).toBe(false);
    expect(cardEvent.defaultPrevented).toBe(false);
    expect(cardPathEvent.defaultPrevented).toBe(false);
    expect(scrollEvent.defaultPrevented).toBe(false);
    expect(buttonEvent.defaultPrevented).toBe(false);
    expect(clicks).toBe(1);
    expect(gutter.children).toHaveLength(0);
    expect(gutter.nextElementSibling).toBe(contentWrapper);
    expect(gutter.classList.contains("h-2.5")).toBe(true);
    expect(gutter.classList.contains("absolute")).toBe(false);
    expect(contentWrapper.classList.contains("min-h-0")).toBe(true);
    expect(contentWrapper.classList.contains("flex-1")).toBe(true);
    expect(contentWrapper.classList.contains("overflow-hidden")).toBe(true);
  });
});
