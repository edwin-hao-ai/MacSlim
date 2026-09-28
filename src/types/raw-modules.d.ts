// vite/client 已经声明过 `*?raw`，但 tsconfig 的 `include` 没有引用它。
// 这里补一份最小声明，让测试能把后端源码当纯文本读进来做跨语言门禁
// （后端发出的 i18n key 必须在中英两侧词典里都有）。
declare module "*?raw" {
  const content: string;
  export default content;
}
