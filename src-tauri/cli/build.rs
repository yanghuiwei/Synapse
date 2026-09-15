// 把 VC 运行时静态链进 reqly-cli，让它在干净机器上不依赖 VC++ 可再发行包。
//
// 与 `mcp/build.rs` 是同一件事、同一个理由（那边有完整说明），这里只留要点：
// `tauri-build` 只给桌面主程序做了静态 CRT，本 package 是独立 crate、不跑它，
// 于是产出的 exe 硬导入 `VCRUNTIME140.dll` —— 干净 Windows 上会在进程入口处直接失败。
//
// 顺带一提，CLI 比 MCP 更容易撞上这个：安装包会把 `reqly-cli.exe` 加进用户 PATH，
// 也就是鼓励用户在自己的终端里直接敲它；MCP 至少还是由客户端按绝对路径拉起。
// 用户在自己的机器上敲一条刚装好的命令、得到一个「缺少 DLL」的系统弹窗，是最难解释的那种失败。

fn main() {
    static_vcruntime::metabuild();
}
