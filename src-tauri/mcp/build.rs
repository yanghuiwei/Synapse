// 把 VC 运行时静态链进 reqly-mcp，让它在干净机器上不依赖 VC++ 可再发行包。
//
// 为什么需要这一件事：Tauri 只给**桌面主程序**做了静态 CRT —— `tauri-build` 的构建脚本会塞进
// `/NODEFAULTLIB:vcruntime.lib` + `/DEFAULTLIB:libvcruntime.lib`（它内嵌了 `static_vcruntime` 的
// 那套逻辑，见 `tauri-build/src/static_vcruntime.rs`）。而本 package 是独立 crate、不跑
// `tauri-build`，于是走 rustc 默认的动态 CRT，产出的 exe **硬导入 `VCRUNTIME140.dll`**：
// 干净 Windows 上 GUI 能起来，这个 sidecar 会在进程入口处直接失败 —— 连错误码都出不来。
//
// 这正好是「拆 crate 拿体积」的连带代价：拆出去的同时也脱离了桌面 package 的构建脚本，
// 凡是 `tauri-build` 顺手做的事都得自己补一遍。
//
// 用的是官方那个 crate，不是手写：它做的是「C 启动文件 + VC 运行时静态、UCRT 仍动态」——
// UCRT（`api-ms-win-crt-*.dll`）是 Windows 自带组件，动态链接才是对的（`+crt-static`
// 会连 UCRT 一起静态化，没必要）。静态化之后的导入表与桌面端主程序**完全同形**，
// 这样三个二进制对目标机的要求就统一了。
//
// 注意：它只作用于本 package 的链接步骤，因此 proc-macro 与 build script 仍是动态 CRT
// —— 不引入「跨 CRT 堆」这类只在 dylib 边界才暴露的问题。

fn main() {
    static_vcruntime::metabuild();
}
