# Changelog

## [0.6.0](https://github.com/yanghuiwei/Synapse/compare/v0.5.0...v0.6.0) (2026-10-13)


### Features

* give each sidebar section its own workspace view ([6d0a682](https://github.com/yanghuiwei/Synapse/commit/6d0a682e8e08a798e0cfe9d9781238daa8f591bd))
* **loadtest:** split new-run and history into tabs ([4117fcf](https://github.com/yanghuiwei/Synapse/commit/4117fcf677778e31bc825bad256603e74ac7112b))
* **theme:** add five accent palettes as an orthogonal theme dimension ([b3cf8b6](https://github.com/yanghuiwei/Synapse/commit/b3cf8b625f63d4ca94fcb3ac2684d7fa0fde21bd))
* **tools:** add a Tools section with a local JWT parser ([c50f4d6](https://github.com/yanghuiwei/Synapse/commit/c50f4d6ca086df05969d6ccb98cb4ccfdcba236b))
* **verify:** 验证 release-please 在死锁解开后会正常开出版本 PR ([#18](https://github.com/yanghuiwei/Synapse/issues/18)) ([8414cfd](https://github.com/yanghuiwei/Synapse/commit/8414cfdd8885c8119a14bcc312f3d997d8571e8d))
* **window:** 自绘标题栏（去系统标题栏 + 整条顶栏可拖 + 窗口按钮） ([6ce9e73](https://github.com/yanghuiwei/Synapse/commit/6ce9e7367a5c853e61a11cd1a6bc30cf161f132e))
* **workspace:** give history and loadtest their own main views ([9c5fac9](https://github.com/yanghuiwei/Synapse/commit/9c5fac9f7d9320fdf69f8138d9c2b44cac0355fb))
* 再次验证 release-please（tag 类型问题） ([e3c88f4](https://github.com/yanghuiwei/Synapse/commit/e3c88f4fc8e5226b286117111f487b63912e1209))
* 死锁解开后验证 release-please 会正常开出版本 PR ([60e0180](https://github.com/yanghuiwei/Synapse/commit/60e01800d687669e4fb7a23ac1e2bb115066db00))
* 验证 autorelease 标签修正后 release-please 恢复 ([862cf6f](https://github.com/yanghuiwei/Synapse/commit/862cf6fbc3346337ba7f53795b8183f030a3865a))


### Bug Fixes

* **ci:** 删掉 squash 合并留下的重复 target 段 ([3293b0e](https://github.com/yanghuiwei/Synapse/commit/3293b0e72b76781ab311ebddd335e61d34e4fb65))
* **ci:** 发布 tag 必须用轻量 tag，附注 tag 会让 release-please 永远判为未打 tag ([49045e9](https://github.com/yanghuiwei/Synapse/commit/49045e98445c8caf761111870a815fc1ccb91af0))
* **ci:** 发布流水线补推 tag，解开 release-please 的闭环死锁 ([6fcc7c2](https://github.com/yanghuiwei/Synapse/commit/6fcc7c22e32bedc5c74a677d72cf0a6576dce798))
* **ci:** 订正 publish.yml 里关于 tag 的错误注释 ([79dfbab](https://github.com/yanghuiwei/Synapse/commit/79dfbab28534ced9b1a2937fee86c0aaa4c36597))
* **ci:** 通用依赖被误塞进 cfg(windows) 段，macOS/Linux 构建全红 ([#16](https://github.com/yanghuiwei/Synapse/issues/16)) ([6d79e69](https://github.com/yanghuiwei/Synapse/commit/6d79e69656b721e46bb91b22cf8cfd53c38d03be))
* **loadtest:** stop restating the run number in the report ([8361ef5](https://github.com/yanghuiwei/Synapse/commit/8361ef53b7e5ec7fcf7132481069f2ba8966d308))
* **response:** keep the pretty/raw toggle usable after switching to raw ([efaff89](https://github.com/yanghuiwei/Synapse/commit/efaff89b5022f75167f2faffcb01640f1728776c))

## [0.5.0](https://github.com/yanghuiwei/Synapse/compare/v0.4.0...v0.5.0) (2026-09-23)


### Features

* **brand:** 全面更名为 Synapse，重写 README 并补齐忽略规则 ([afcc79d](https://github.com/yanghuiwei/Synapse/commit/afcc79d78ad0b06a000739a6fa8bc75b1d93ef8c))
* **brand:** 全面更名为 Synapse，重写 README 并补齐忽略规则 ([79b23be](https://github.com/yanghuiwei/Synapse/commit/79b23be5af0097ffce2ace68731348f585f2cdb5))


### Bug Fixes

* **ci:** drop stale brand prefix from Release titles ([2f84a18](https://github.com/yanghuiwei/Synapse/commit/2f84a1815125817a3cf37b05f404ae31184c48f6))
* **ci:** drop stale brand prefix from Release titles ([1dae4d4](https://github.com/yanghuiwei/Synapse/commit/1dae4d4ab49966172c6896efdeee7e2db8353d20))

## [0.4.0](https://github.com/yanghuiwei/Synapse/compare/v0.3.0...v0.4.0) (2026-09-23)


### ⚠ BREAKING CHANGES

* 迁移 007 不可逆（DROP COLUMN auth），回退只能从备份恢复。

### Features

* CLI 进 PATH、HAR 1.2 导入器，并修掉 9 处旧数据目录口径 ([7fcf4ae](https://github.com/yanghuiwei/Synapse/commit/7fcf4ae59dccef4b35eb8d1472130f152839ba0a))
* **installer:** brand NSIS installer with Synapse sidebar and header ([fe2e169](https://github.com/yanghuiwei/Synapse/commit/fe2e169f9e7a81ff0261c7012a892926b0d51d0a))
* P1–P6 全部落地（集合 / 历史 / 变量与 Token / 导入器 / 压测） ([db9842b](https://github.com/yanghuiwei/Synapse/commit/db9842b4faa90c453e267a3e4654b44d24d729be))
* P7 MCP Server 落地，并修一批交互缺陷 ([0bc8348](https://github.com/yanghuiwei/Synapse/commit/0bc83488557bffcc67ae5ef9a35a020d9a708ead))
* sidecar 静态链接 VC 运行时，并重写 README 的依赖与打包说明 ([5502a5e](https://github.com/yanghuiwei/Synapse/commit/5502a5ef7f8d8babef0cf7dd13a3bc0743200433))
* Token 来源改为内置系统分类页内直配，并入 ApiPost 导入器支持 ([88874c2](https://github.com/yanghuiwei/Synapse/commit/88874c254a42ba3338e55a92d34f7c363f71f247))
* Token 源下沉到每个接口（008/009）+ 历史留一份当时的响应（010） ([be8f646](https://github.com/yanghuiwei/Synapse/commit/be8f646740055add5805f0c9585b146fa6404859))
* Token 缓存与倒计时按「取 token 的配置」共享，而不是按来源记录 ([daccb42](https://github.com/yanghuiwei/Synapse/commit/daccb42a2ef5ef038738e2b392f7657b6ed1aa10))
* 侧栏新建收敛为组合按钮，并打通「草稿存进分类」这条断链 ([9f15a61](https://github.com/yanghuiwei/Synapse/commit/9f15a61813191cef50d6e144cfc17ab4f0ffb2a1))
* 压测历史补上「当时那一轮是怎么打的」 ([056297d](https://github.com/yanghuiwei/Synapse/commit/056297da4413cca90d5e4815c6d2485e00e6db8e))
* 取 Token 接口改成自定义下拉 + Ctrl+K 命令面板 ([c485e56](https://github.com/yanghuiwei/Synapse/commit/c485e56a4a7cbd8e843b024e1504716172e3fab6))
* 取消「授权」概念 —— 凭据就是请求头里的一行（迁移 007） ([15dc2b2](https://github.com/yanghuiwei/Synapse/commit/15dc2b2a530af8d0bede7ae281b6dffc5f327669))
* 应用标识改成 com.reqly.desktop，配套老数据一次性迁移 ([26ad3eb](https://github.com/yanghuiwei/Synapse/commit/26ad3ebdb5d72ac9a3ffd93afca59d790e604eb5))
* 拆 crate 加 CLI 通道，MCP / CLI 进安装包 ([b194924](https://github.com/yanghuiwei/Synapse/commit/b194924abe5a06d60d41db0833d25ae0574f95ea))
* 支持 multipart/form-data 请求体与二进制响应流落盘，并收尾 Token 表单 ([9ffeb20](https://github.com/yanghuiwei/Synapse/commit/9ffeb20a4f5de59acca2fd417cfc7e9b4d5a143c))


### Bug Fixes

* **ci:** correct release-please extra-files schema and tauri-action i… ([4282cfd](https://github.com/yanghuiwei/Synapse/commit/4282cfdc5fb00e595a24a588f146421b737e6fa2))
* **ci:** correct release-please extra-files schema and tauri-action input name ([1c2d7a7](https://github.com/yanghuiwei/Synapse/commit/1c2d7a7372879859b1fab9ed183c6d10326a77fe))
* **ci:** create release-please GitHub Releases as drafts ([4d9b675](https://github.com/yanghuiwei/Synapse/commit/4d9b6757c042ba7a853a67a4368ee238d64e14c4))
* **ci:** create release-please GitHub Releases as drafts ([201104e](https://github.com/yanghuiwei/Synapse/commit/201104e442c5613c56d904e45f62f25a81bc10c6))
* **ci:** let publish own the Release object, release-please owns the tag ([263c939](https://github.com/yanghuiwei/Synapse/commit/263c9396855c75b31c5a5f550183635ef4e59340))
* **ci:** let publish own the Release object, release-please owns the tag ([97ce4c5](https://github.com/yanghuiwei/Synapse/commit/97ce4c54e5654312273bee683baef27502a0668c))
* **collections:** 搜索时命中的请求不再被所属分类吞掉 ([d385067](https://github.com/yanghuiwei/Synapse/commit/d385067da45148cabac0c069ed353d168b20aaf6))
* **installer:** 缩短 PATH 超限提示，并补一个 PATH 清理脚本（§20.8/§20.9） ([38086c2](https://github.com/yanghuiwei/Synapse/commit/38086c22209afa85b83f877ab791a91af93b0954))
* 修复内联 body 被静默丢弃，并补两条 CLI 管理类入口 ([b5ab57d](https://github.com/yanghuiwei/Synapse/commit/b5ab57dc6c3f2af5e038ff6b74321ae60e6b796e))
* 表格长值改为折行显示，并修 urlencoded 换行分隔、切类型丢内容与 IconButton 图标三处静默失效 ([510a28b](https://github.com/yanghuiwei/Synapse/commit/510a28b7e7d86d689e5aa6203c1f5708f60be51f))

## [0.3.0](https://github.com/yanghuiwei/Synapse/compare/v0.2.1...v0.3.0) (2026-09-23)


### ⚠ BREAKING CHANGES

* 迁移 007 不可逆（DROP COLUMN auth），回退只能从备份恢复。

### Features

* CLI 进 PATH、HAR 1.2 导入器，并修掉 9 处旧数据目录口径 ([7fcf4ae](https://github.com/yanghuiwei/Synapse/commit/7fcf4ae59dccef4b35eb8d1472130f152839ba0a))
* **installer:** brand NSIS installer with Synapse sidebar and header ([fe2e169](https://github.com/yanghuiwei/Synapse/commit/fe2e169f9e7a81ff0261c7012a892926b0d51d0a))
* P1–P6 全部落地（集合 / 历史 / 变量与 Token / 导入器 / 压测） ([db9842b](https://github.com/yanghuiwei/Synapse/commit/db9842b4faa90c453e267a3e4654b44d24d729be))
* P7 MCP Server 落地，并修一批交互缺陷 ([0bc8348](https://github.com/yanghuiwei/Synapse/commit/0bc83488557bffcc67ae5ef9a35a020d9a708ead))
* sidecar 静态链接 VC 运行时，并重写 README 的依赖与打包说明 ([5502a5e](https://github.com/yanghuiwei/Synapse/commit/5502a5ef7f8d8babef0cf7dd13a3bc0743200433))
* Token 来源改为内置系统分类页内直配，并入 ApiPost 导入器支持 ([88874c2](https://github.com/yanghuiwei/Synapse/commit/88874c254a42ba3338e55a92d34f7c363f71f247))
* Token 源下沉到每个接口（008/009）+ 历史留一份当时的响应（010） ([be8f646](https://github.com/yanghuiwei/Synapse/commit/be8f646740055add5805f0c9585b146fa6404859))
* Token 缓存与倒计时按「取 token 的配置」共享，而不是按来源记录 ([daccb42](https://github.com/yanghuiwei/Synapse/commit/daccb42a2ef5ef038738e2b392f7657b6ed1aa10))
* 侧栏新建收敛为组合按钮，并打通「草稿存进分类」这条断链 ([9f15a61](https://github.com/yanghuiwei/Synapse/commit/9f15a61813191cef50d6e144cfc17ab4f0ffb2a1))
* 压测历史补上「当时那一轮是怎么打的」 ([056297d](https://github.com/yanghuiwei/Synapse/commit/056297da4413cca90d5e4815c6d2485e00e6db8e))
* 取 Token 接口改成自定义下拉 + Ctrl+K 命令面板 ([c485e56](https://github.com/yanghuiwei/Synapse/commit/c485e56a4a7cbd8e843b024e1504716172e3fab6))
* 取消「授权」概念 —— 凭据就是请求头里的一行（迁移 007） ([15dc2b2](https://github.com/yanghuiwei/Synapse/commit/15dc2b2a530af8d0bede7ae281b6dffc5f327669))
* 应用标识改成 com.reqly.desktop，配套老数据一次性迁移 ([26ad3eb](https://github.com/yanghuiwei/Synapse/commit/26ad3ebdb5d72ac9a3ffd93afca59d790e604eb5))
* 拆 crate 加 CLI 通道，MCP / CLI 进安装包 ([b194924](https://github.com/yanghuiwei/Synapse/commit/b194924abe5a06d60d41db0833d25ae0574f95ea))
* 支持 multipart/form-data 请求体与二进制响应流落盘，并收尾 Token 表单 ([9ffeb20](https://github.com/yanghuiwei/Synapse/commit/9ffeb20a4f5de59acca2fd417cfc7e9b4d5a143c))


### Bug Fixes

* **ci:** correct release-please extra-files schema and tauri-action i… ([4282cfd](https://github.com/yanghuiwei/Synapse/commit/4282cfdc5fb00e595a24a588f146421b737e6fa2))
* **ci:** correct release-please extra-files schema and tauri-action input name ([1c2d7a7](https://github.com/yanghuiwei/Synapse/commit/1c2d7a7372879859b1fab9ed183c6d10326a77fe))
* **ci:** create release-please GitHub Releases as drafts ([4d9b675](https://github.com/yanghuiwei/Synapse/commit/4d9b6757c042ba7a853a67a4368ee238d64e14c4))
* **ci:** create release-please GitHub Releases as drafts ([201104e](https://github.com/yanghuiwei/Synapse/commit/201104e442c5613c56d904e45f62f25a81bc10c6))
* **collections:** 搜索时命中的请求不再被所属分类吞掉 ([d385067](https://github.com/yanghuiwei/Synapse/commit/d385067da45148cabac0c069ed353d168b20aaf6))
* **installer:** 缩短 PATH 超限提示，并补一个 PATH 清理脚本（§20.8/§20.9） ([38086c2](https://github.com/yanghuiwei/Synapse/commit/38086c22209afa85b83f877ab791a91af93b0954))
* 修复内联 body 被静默丢弃，并补两条 CLI 管理类入口 ([b5ab57d](https://github.com/yanghuiwei/Synapse/commit/b5ab57dc6c3f2af5e038ff6b74321ae60e6b796e))
* 表格长值改为折行显示，并修 urlencoded 换行分隔、切类型丢内容与 IconButton 图标三处静默失效 ([510a28b](https://github.com/yanghuiwei/Synapse/commit/510a28b7e7d86d689e5aa6203c1f5708f60be51f))

## [0.2.1](https://github.com/yanghuiwei/Synapse/compare/v0.2.0...v0.2.1) (2026-09-23)


### Bug Fixes

* **ci:** create release-please GitHub Releases as drafts ([4d9b675](https://github.com/yanghuiwei/Synapse/commit/4d9b6757c042ba7a853a67a4368ee238d64e14c4))
* **ci:** create release-please GitHub Releases as drafts ([201104e](https://github.com/yanghuiwei/Synapse/commit/201104e442c5613c56d904e45f62f25a81bc10c6))

## [0.2.0](https://github.com/yanghuiwei/Synapse/compare/v0.1.0...v0.2.0) (2026-09-23)


### ⚠ BREAKING CHANGES

* 迁移 007 不可逆（DROP COLUMN auth），回退只能从备份恢复。

### Features

* CLI 进 PATH、HAR 1.2 导入器，并修掉 9 处旧数据目录口径 ([7fcf4ae](https://github.com/yanghuiwei/Synapse/commit/7fcf4ae59dccef4b35eb8d1472130f152839ba0a))
* **installer:** brand NSIS installer with Synapse sidebar and header ([fe2e169](https://github.com/yanghuiwei/Synapse/commit/fe2e169f9e7a81ff0261c7012a892926b0d51d0a))
* P1–P6 全部落地（集合 / 历史 / 变量与 Token / 导入器 / 压测） ([db9842b](https://github.com/yanghuiwei/Synapse/commit/db9842b4faa90c453e267a3e4654b44d24d729be))
* P7 MCP Server 落地，并修一批交互缺陷 ([0bc8348](https://github.com/yanghuiwei/Synapse/commit/0bc83488557bffcc67ae5ef9a35a020d9a708ead))
* sidecar 静态链接 VC 运行时，并重写 README 的依赖与打包说明 ([5502a5e](https://github.com/yanghuiwei/Synapse/commit/5502a5ef7f8d8babef0cf7dd13a3bc0743200433))
* Token 来源改为内置系统分类页内直配，并入 ApiPost 导入器支持 ([88874c2](https://github.com/yanghuiwei/Synapse/commit/88874c254a42ba3338e55a92d34f7c363f71f247))
* Token 源下沉到每个接口（008/009）+ 历史留一份当时的响应（010） ([be8f646](https://github.com/yanghuiwei/Synapse/commit/be8f646740055add5805f0c9585b146fa6404859))
* Token 缓存与倒计时按「取 token 的配置」共享，而不是按来源记录 ([daccb42](https://github.com/yanghuiwei/Synapse/commit/daccb42a2ef5ef038738e2b392f7657b6ed1aa10))
* 侧栏新建收敛为组合按钮，并打通「草稿存进分类」这条断链 ([9f15a61](https://github.com/yanghuiwei/Synapse/commit/9f15a61813191cef50d6e144cfc17ab4f0ffb2a1))
* 压测历史补上「当时那一轮是怎么打的」 ([056297d](https://github.com/yanghuiwei/Synapse/commit/056297da4413cca90d5e4815c6d2485e00e6db8e))
* 取 Token 接口改成自定义下拉 + Ctrl+K 命令面板 ([c485e56](https://github.com/yanghuiwei/Synapse/commit/c485e56a4a7cbd8e843b024e1504716172e3fab6))
* 取消「授权」概念 —— 凭据就是请求头里的一行（迁移 007） ([15dc2b2](https://github.com/yanghuiwei/Synapse/commit/15dc2b2a530af8d0bede7ae281b6dffc5f327669))
* 应用标识改成 com.reqly.desktop，配套老数据一次性迁移 ([26ad3eb](https://github.com/yanghuiwei/Synapse/commit/26ad3ebdb5d72ac9a3ffd93afca59d790e604eb5))
* 拆 crate 加 CLI 通道，MCP / CLI 进安装包 ([b194924](https://github.com/yanghuiwei/Synapse/commit/b194924abe5a06d60d41db0833d25ae0574f95ea))
* 支持 multipart/form-data 请求体与二进制响应流落盘，并收尾 Token 表单 ([9ffeb20](https://github.com/yanghuiwei/Synapse/commit/9ffeb20a4f5de59acca2fd417cfc7e9b4d5a143c))


### Bug Fixes

* **ci:** correct release-please extra-files schema and tauri-action i… ([4282cfd](https://github.com/yanghuiwei/Synapse/commit/4282cfdc5fb00e595a24a588f146421b737e6fa2))
* **ci:** correct release-please extra-files schema and tauri-action input name ([1c2d7a7](https://github.com/yanghuiwei/Synapse/commit/1c2d7a7372879859b1fab9ed183c6d10326a77fe))
* **collections:** 搜索时命中的请求不再被所属分类吞掉 ([d385067](https://github.com/yanghuiwei/Synapse/commit/d385067da45148cabac0c069ed353d168b20aaf6))
* **installer:** 缩短 PATH 超限提示，并补一个 PATH 清理脚本（§20.8/§20.9） ([38086c2](https://github.com/yanghuiwei/Synapse/commit/38086c22209afa85b83f877ab791a91af93b0954))
* 修复内联 body 被静默丢弃，并补两条 CLI 管理类入口 ([b5ab57d](https://github.com/yanghuiwei/Synapse/commit/b5ab57dc6c3f2af5e038ff6b74321ae60e6b796e))
* 表格长值改为折行显示，并修 urlencoded 换行分隔、切类型丢内容与 IconButton 图标三处静默失效 ([510a28b](https://github.com/yanghuiwei/Synapse/commit/510a28b7e7d86d689e5aa6203c1f5708f60be51f))
