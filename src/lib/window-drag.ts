import { getCurrentWindow } from "@tauri-apps/api/window";

const NON_DRAG_SELECTOR = [
  "button",
  "input",
  "textarea",
  "select",
  "a",
  "label",
  "summary",
  ".card",
  "li",
  "[data-no-drag]",
  "[contenteditable]:not([contenteditable='false'])",
].join(",");

const isContentEditable = (element: Element) => {
  let current: Element | null = element;
  while (current) {
    const contentEditable = (current as HTMLElement).contentEditable;
    if (contentEditable === "true" || contentEditable === "plaintext-only") {
      return true;
    }
    current = current.parentElement;
  }
  return false;
};

/**
 * 元素**自身**（不含后代）是否带文字。
 *
 * 这是「背景可拖 / 文字可拖选」这条分界线。必须只看自身文本节点：如果看
 * `textContent`，那么包着整棵内容树的内容容器会因为"后代里有字"而永远判定为
 * 文字区，于是整个内容区都拖不动 —— 正好是 P0 重构留下的那个毛病。
 */
const hasOwnText = (element: Element) => {
  for (const node of Array.from(element.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE && /\S/.test(node.textContent ?? "")) {
      return true;
    }
  }
  return false;
};

export const isSafeWindowDragTarget = (target: EventTarget | null) => {
  if (!(target instanceof Element)) return false;
  if (target.closest(NON_DRAG_SELECTOR) !== null) return false;
  if (isContentEditable(target)) return false;
  // 压在文字上时让位给拖选：拖窗口和拖选文本是同一个手势，同一块区域上不可能
  // 兼得，所以只在真正的空白背景上接管。滚轮滚动不受影响（那是 wheel 事件，
  // 不经过 mousedown）。
  return !hasOwnText(target);
};

export const handleWindowDrag = (event: MouseEvent) => {
  if (event.button !== 0 || !isSafeWindowDragTarget(event.target)) return;
  event.preventDefault();
  void getCurrentWindow()
    .startDragging()
    .catch((error: unknown) => {
      console.error("拖动窗口失败", error);
    });
};
