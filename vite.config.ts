import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [solid(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    clearMocks: true,
    restoreMocks: true,
    // 这些是走完整组件树的集成测试，单条里有十几步 await。默认 5s 在
    // 机器一忙（并行跑 22 个文件、或别的项目在编译）就会超时，而它们其实
    // 全都通过 —— 单跑 4 秒内完成。
    //
    // 踩过两次：一次是别的项目把 load average 拉到 177，一次是全量并行时
    // transform 阶段排队。两次都是 `Test timed out in 5000ms`，没有一条真的
    // 失败。把超时当成需要「重跑到绿」的问题，会把 flake 洗掉而不是修掉，
    // 下次照样随机红 —— 所以这里直接给足余量。
    testTimeout: 20_000,
    hookTimeout: 20_000,
  },
}));
