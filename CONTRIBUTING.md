# 贡献指南

感谢你考虑为 Synapse 做贡献。这份文档说明怎么把改动递进来，以及哪些约定能少走弯路。

## 我想要什么样的 PR

**说清「为什么」，而不只是「改了什么」。** 这个仓库的代码注释密度不低，很多地方记着
「为什么这么写」——尤其是几处本地验不出来、只有真实环境才暴露的问题。延续这个习惯，
你的改动才不会在半年后变成没人敢碰的谜团。

一个具体的例子：如果你的 PR 修了一个 bug，请说明**它是怎么被发现的**。
是某个环境独有的现象？是某种输入组合触发的？这条信息往往比修复本身更有价值，
因为它决定了后来的人怎么判断这个问题是否已经彻底解决。

## 开发环境

| 依赖 | 版本 |
|---|---|
| Node.js | `^20.19.0` 或 `>=22.12.0` |
| pnpm | 10 |
| Rust | stable |
| MSVC 生成工具 | Windows 上由 Visual Studio 2022 / VS Build Tools 提供（勾选「使用 C++ 的桌面开发」） |

```bash
git clone https://github.com/yanghuiwei/Synapse.git
cd Synapse
pnpm install

pnpm tauri dev       # 开发
pnpm tauri build     # 出安装包
```

## 本地要跑什么

**每次提交前至少跑一遍这些** —— CI 也会跑同样的门禁，本地先过能省一轮往返：

```bash
# Rust：格式、静态检查、测试
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -p reqly-core -p reqly-cli -p reqly-mcp --all-targets
cargo test --manifest-path src-tauri/Cargo.toml -p reqly-core -p reqly-cli -p reqly-mcp

# 前端
pnpm exec tsc --noEmit
pnpm build
```

**关于三平台的一个坑（重要）**：CI 在 Linux / macOS 上只跑 `reqly-core`、`reqly-cli`、
`reqly-mcp` 这三个 crate，桌面端 `reqly` 只在 Windows 上跑。原因是 `reqly` 链 tauri，
编译需要 MSVC 链接器与 WebView2 相关头文件，在 Linux 镜像上起不来。

由此推出一条**你必须自己守的纪律**：那三个 crate 里不要引入只在 Windows 上才成立的依赖。
一旦引入，Linux CI 会报出一片 `unresolved import serde` 式的红，而**你本机 Windows 一切正常**——
这种问题本地永远验不出来。具体案例见 `src-tauri/Cargo.toml` 里
`[target.'cfg(windows)'.dependencies]` 那一段上方的注释。

## 提交信息

跟随 [Conventional Commits](https://www.conventionalcommits.org/)：

```
feat(loadtest): 报告页支持导出 CSV
fix(import): HAR 里 multipart 丢文件时不静默丢弃
refactor(core): 把错误码归到 error.rs 一处
docs: 补充压测分位算法的说明
```

**不要手改版本号和 `CHANGELOG.md`** —— 发版由 [release-please](https://github.com/googleapis/release-please)
从提交信息自动生成。`package.json`、`tauri.conf.json`、四个 `Cargo.toml` 里的
`# x-release-please-version` 注释是它的锚点，动了会破坏发版。

## 分支与 PR

```bash
git checkout -b feat/your-feature
# ...
git commit -s -m "feat(scope): 一句话说清做了什么"
git push origin feat/your-feature
```

用 `-s`（sign-off）签名，声明你有权提交这段代码。

然后开 PR。描述里写清三件事：**改了什么、为什么这么改、怎么验证的**。

## 代码约定

仓库里有几条不成文的规矩，改动相关文件时请遵守：

- **错误对外只给码，不给文案。** `core/src/error.rs` 里是稳定错误码（`E_TIMEOUT` 等），
  文案爱怎么改怎么改，但不改码、不增含义。
- **凭据打码名单只有一份。** 在 `core/src/redact.rs`，不要在别处另写一套。
- **注释解释「为什么」，不复述「做了什么」。** 代码本身已经说了做了什么。
- **中文注释。** 这是当前代码库的既定语言，与界面语言一致。

## 报告 Bug

开 Issue 时请附上：操作系统、Synapse 版本、复现步骤、实际结果与预期结果。
如果是「AI 通过 MCP 改了数据但界面没更新」这类，附上 MCP 侧的调用记录会快很多。

## 需要讨论的大改动

**先开 Issue 讨论，再写代码。** 尤其是涉及以下几项的：

- 新的网络协议（gRPC / WebSocket 等）——设计文档里明确列为「架构预留、本期不做」
- 数据模型变更 —— 迁移不可逆，且写错会波及用户已有的库
- 破坏性 CLI / MCP 接口变更 —— 已有用户和脚本依赖现有形状

省下双方的时间。

## 行为准则

参与即表示同意遵守 [行为准则](CODE_OF_CONDUCT.md)。
