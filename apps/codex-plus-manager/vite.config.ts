import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    // 显式钉住构建目标（#2753）：macOS 上用的是系统自带的 WebKit，最低支持的 macOS 12 出厂是 15.0 版，
    // 旧 Intel Mac 常年停在 15.x。不写的话跟着 Vite 默认值走，升级 Vite 后默认目标会抬高到更新的浏览器基线。
    // safari14 是能落到的最低一档：代码里有 BigInt（模型窗口换算），14 以下没有 BigInt，降语法也救不回来。
    // 运行时 API（Object.hasOwn、Array.prototype.at、<dialog>）构建降不了，在 src/legacy-webkit.ts 里兜底；
    // 正则后行断言也降不了，源码里不要用（legacy-webkit.test.ts 守着）。
    target: ["es2020", "safari14", "chrome87", "edge88", "firefox78"],
    cssTarget: ["safari14", "chrome87", "edge88", "firefox78"],
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
});
