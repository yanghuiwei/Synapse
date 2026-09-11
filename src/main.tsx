import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// 样式引入顺序有讲究：
//   1. theme.css   —— 定义原始令牌（--canvas / --ink / ...）
//   2. tailwind.css —— Tailwind 本体 + preflight + @theme inline 映射
//   3. global.css  —— 我们的基础样式，必须排在 preflight 之后才能覆盖它
import "./styles/theme.css";
import "./styles/tailwind.css";
import "./styles/global.css";

import App from "./App";
import { I18nProvider } from "./lib/i18n";
import { watchSystemTheme } from "./lib/theme";

// 在渲染前挂上系统主题监听，避免首帧之后再补一次重绘
watchSystemTheme();

const container = document.getElementById("root");
if (!container) {
  throw new Error("找不到 #root 容器，检查 index.html");
}

createRoot(container).render(
  <StrictMode>
    <I18nProvider>
      <App />
    </I18nProvider>
  </StrictMode>,
);
