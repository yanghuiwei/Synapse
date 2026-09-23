# Changelog

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
