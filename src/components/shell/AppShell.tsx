import { type Component, type JSX } from "solid-js";
import SafeDragSurface from "@/components/shell/SafeDragSurface";

type AppShellProps = {
  sidebar: JSX.Element;
  toolbar: JSX.Element;
  children: JSX.Element;
};

const AppShell: Component<AppShellProps> = (props) => (
  <div class="flex h-full bg-[rgb(var(--bg-app))/var(--bg-app-alpha)]">
    {props.sidebar}
    <main class="flex min-w-0 flex-1 flex-col">
      <SafeDragSurface class="flex h-12 items-center border-b border-black/5 px-6 dark:border-white/5">
        {props.toolbar}
      </SafeDragSurface>
      <SafeDragSurface
        directTargetOnly
        data-testid="content-drag-gutter"
        class="h-2.5 shrink-0"
      />
      {/*
        整个内容区都是拖动面。`handleWindowDrag` 的排除规则（window-drag.ts 的
        `NON_DRAG_SELECTOR`）在这里起作用：`.card` / `li` / `button` / `input` /
        `a` / `label` / `[data-no-drag]` 都不可拖，所以卡片**内部**（路径文本、
        进程命令、条目文字）仍能拖选复制，拖不动的只有卡片之间的间隙与外边距。

        P0 重构时这一层被换成了没有 handler 的普通 div，拖动区缩到 48px 工具栏
        + 10px 细条；`AppShell.test.tsx` 当时把「内容区不拖」当成设计意图断言
        了下来，于是退化过了 CI。恢复时不要只改代码不改断言。
      */}
      <SafeDragSurface
        data-testid="content-wrapper"
        class="min-h-0 flex-1 overflow-hidden"
      >
        {props.children}
      </SafeDragSurface>
    </main>
  </div>
);

export default AppShell;
