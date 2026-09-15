#!/usr/bin/env node
// 把两个 sidecar（`reqly-mcp` / `reqly-cli`）编好，搬到 `src-tauri/binaries/` 并改名成
// `-$TARGET_TRIPLE` 形式 —— 这是 `tauri.conf.json` 的 `bundle.externalBin` 唯一认的格式。
//
// ## 为什么必须是脚本，不能靠手工拷
//
// `externalBin` 指向的文件在**构建期**就要求存在，少一个 `tauri build` 直接失败。
// 手工拷贝的失败方式很难看：在本地能过（`cargo build` 会把同 package 的所有 bin 放进
// `target/release/`），到 clean clone / CI 上才炸，而且报错只说"找不到 binaries/xxx"。
// 所以这一步挂在 `beforeBuildCommand` 上，`pnpm tauri build` 一条命令自带它。
//
// ## 为什么两个都要
//
// 桌面端是**按 `current_exe()` 的同级目录**去找 `reqly-mcp.exe` 的
//（`src/commands/mcp.rs::mcp_command_path`），MCP 面板会把那个绝对路径交给用户粘进
// WorkBuddy / Hermes 的配置。开发态成立（同 package 的 bin 都落在一个目录），
// **打包后不成立**：Tauri 只收主 bin。MCP 不进包 → 面板给出的路径在安装版上不存在，
// `command_exists` 恒为 false，H7 那条端到端验收就无从谈起。
//
// ## 交叉编译
//
// 传 `--target <triple>` 时产物在 `target/<triple>/release/`，后缀也用那个 triple ——
// 两者必须一致，否则装上去的是给当前主机编的二进制。
//
// 用法：
//   node scripts/stage-sidecars.mjs
//   node scripts/stage-sidecars.mjs --target aarch64-pc-windows-msvc

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_DIR = join(REPO_ROOT, "src-tauri");
// 相对 `tauri.conf.json`（也就是 `src-tauri/`），别改成仓库根下的路径
const STAGING_DIR = join(TAURI_DIR, "binaries");
const EXE_SUFFIX = process.platform === "win32" ? ".exe" : "";

/** 要进安装包的两个 sidecar，名字 = Cargo 里的 package 名 = 产出的 exe 名 */
const SIDECARS = ["reqly-mcp", "reqly-cli"];

function readOption(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} 后面要跟一个值`);
  }
  return value;
}

/** 不传 `--target` 时用 rustc 的 host —— 也就是"本机默认编出来的那个" */
function hostTriple() {
  const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = output.split(/\r?\n/).find((row) => row.startsWith("host: "));
  if (!line) throw new Error("rustc -vV 的输出里没有 host 行");
  return line.slice("host: ".length).trim();
}

function formatBytes(bytes) {
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

const target = readOption("--target") ?? hostTriple();

console.log(`[sidecar] 目标平台 ${target}`);
console.log(`[sidecar] 编译 ${SIDECARS.join(" / ")} …`);

const buildArgs = [
  "build",
  "--release",
  "--manifest-path",
  join(TAURI_DIR, "Cargo.toml"),
];
for (const name of SIDECARS) buildArgs.push("-p", name);
if (readOption("--target")) buildArgs.push("--target", target);

execFileSync("cargo", buildArgs, { cwd: REPO_ROOT, stdio: "inherit" });

const releaseDir = join(TAURI_DIR, "target", ...(readOption("--target") ? [target] : []), "release");
mkdirSync(STAGING_DIR, { recursive: true });

let failed = false;

for (const name of SIDECARS) {
  const source = join(releaseDir, `${name}${EXE_SUFFIX}`);
  if (!existsSync(source)) {
    console.error(`[sidecar] ✗ 没有产物：${source}`);
    failed = true;
    continue;
  }

  const stagedName = `${name}-${target}${EXE_SUFFIX}`;
  const stagedPath = join(STAGING_DIR, stagedName);
  copyFileSync(source, stagedPath);

  const size = statSync(stagedPath).size;
  console.log(
    `[sidecar] ✓ ${stagedName}  ${formatBytes(size)}  → 安装后会是 ${name}${EXE_SUFFIX}`,
  );
}

if (failed) {
  // 出声退出：静默留下一个缺文件的 staging 目录，只会在 `tauri build` 那里以
  // 一句「找不到 binaries/xxx」的形式回来，那时已经离原因很远了
  process.exit(1);
}

console.log(`[sidecar] 就绪：${STAGING_DIR}`);
