# sidecar 中转目录

`tauri.conf.json` 的 `bundle.externalBin` 指向这里，但**文件不在这里** ——
它们由 `pnpm sidecar:stage`（已挂在 `beforeBuildCommand` 上，`pnpm tauri build` 会自动跑）
从 `target/release/` 复制过来，并改名成 `<name>-$TARGET_TRIPLE.exe`。

所以这个目录在仓库里是空的，跑过一次构建才会有东西。别手工往里拷 exe：
名字少一个 triple 后缀，`tauri build` 就找不到它。
