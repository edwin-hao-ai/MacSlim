import { beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import {
  handleWindowDrag,
  isSafeWindowDragTarget,
} from "@/lib/window-drag";

const dispatchMouseDown = (target: Element, button = 0) => {
  const event = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    button,
  });
  target.dispatchEvent(event);
  return event;
};

describe("window drag", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    tauriMocks.startDragging.mockClear();
  });

  it("starts dragging for a safe left-button target", () => {
    const surface = document.createElement("div");
    surface.addEventListener("mousedown", handleWindowDrag);
    document.body.append(surface);

    const event = dispatchMouseDown(surface);

    expect(event.defaultPrevented).toBe(true);
    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
  });

  it("ignores non-primary buttons", () => {
    const surface = document.createElement("div");
    surface.addEventListener("mousedown", handleWindowDrag);
    document.body.append(surface);

    const event = dispatchMouseDown(surface, 1);

    expect(event.defaultPrevented).toBe(false);
    expect(tauriMocks.startDragging).not.toHaveBeenCalled();
  });

  it.each([
    "button",
    "input",
    "textarea",
    "select",
    "a",
    "label",
    "summary",
    "li",
  ])("rejects interactive selector %s", (tagName) => {
    const element = document.createElement(tagName);
    document.body.append(element);
    expect(isSafeWindowDragTarget(element)).toBe(false);
  });

  it("rejects cards and explicit no-drag elements", () => {
    const card = document.createElement("div");
    card.className = "card";
    const noDrag = document.createElement("div");
    noDrag.dataset.noDrag = "true";
    document.body.append(card, noDrag);

    expect(isSafeWindowDragTarget(card)).toBe(false);
    expect(isSafeWindowDragTarget(noDrag)).toBe(false);
  });

  it("rejects contenteditable elements", () => {
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    document.body.append(editable);

    expect(isSafeWindowDragTarget(editable)).toBe(false);
  });

  it("allows dragging for contenteditable=false targets", () => {
    const target = document.createElement("div");
    target.setAttribute("contenteditable", "false");
    target.addEventListener("mousedown", handleWindowDrag);
    document.body.append(target);

    const event = dispatchMouseDown(target);

    expect(isSafeWindowDragTarget(target)).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
  });

  // 「背景可拖 / 文字可拖选」的分界线。拖窗口和拖选文本是同一个手势，同一块
  // 区域上不可能兼得，所以压在文字上时必须让位 —— 否则用户就没法选中路径去复制。
  describe("文字让位给拖选", () => {
    it("元素自身带文字时不拖（保住拖选）", () => {
      const label = document.createElement("span");
      label.textContent = "/Users/edwinhao/Library/Caches/npm";
      document.body.append(label);

      expect(isSafeWindowDragTarget(label)).toBe(false);
    });

    it("只有空白与换行时仍然可拖", () => {
      const gutter = document.createElement("div");
      gutter.append(document.createTextNode("  \n "));
      document.body.append(gutter);

      expect(isSafeWindowDragTarget(gutter)).toBe(true);
    });

    it("自身无文字但后代有文字时仍可拖 —— 这是内容区能拖的关键", () => {
      // 只看**自身**文本节点：若看 textContent，包着整棵内容树的容器会因为
      // "后代里有字"而永远不可拖，整个内容区的拖动又会废掉（P0 遗留的毛病）。
      const contentWrapper = document.createElement("div");
      const inner = document.createElement("div");
      inner.textContent = "缓存清理";
      contentWrapper.append(inner);
      document.body.append(contentWrapper);

      expect(isSafeWindowDragTarget(contentWrapper)).toBe(true);
    });
  });
});
