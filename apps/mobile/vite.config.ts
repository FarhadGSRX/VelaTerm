import { defineConfig } from "vite";
import { readFileSync } from "node:fs";

const { version } = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));
// 构建时固定时间，重新打开 App 时保持不变，便于识别实际安装的包。
const builtAt = new Intl.DateTimeFormat("sv-SE", {
  timeZone: "Asia/Shanghai", year: "numeric", month: "2-digit", day: "2-digit",
  hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false,
}).format(new Date());
export default defineConfig({
  define: {
    __MOBILE_VERSION__: JSON.stringify(version),
    __MOBILE_BUILD_TIME__: JSON.stringify(builtAt),
  },
  server: { port: 41571, strictPort: true },
  preview: { port: 41571, strictPort: true },
});
