# Changelog

## [0.8.0](https://github.com/yanghuiwei/Synapse/compare/v0.7.1...v0.8.0) (2026-10-21)


### ⚠ BREAKING CHANGES

* 迁移 007 不可逆（DROP COLUMN auth），回退只能从备份恢复。

### Features

* **brand:** 全面更名为 Synapse，重写 README 并补齐忽略规则 ([b71e56a](https://github.com/yanghuiwei/Synapse/commit/b71e56a86e8f8723caaf9e3264c51b48235d9163))
* **brand:** 全面更名为 Synapse，重写 README 并补齐忽略规则 ([ad33ad8](https://github.com/yanghuiwei/Synapse/commit/ad33ad8b0fd34893121084888ab42b671809594e))
* CLI 进 PATH、HAR 1.2 导入器，并修掉 9 处旧数据目录口径 ([3e28dff](https://github.com/yanghuiwei/Synapse/commit/3e28dff2ce2b2ba64c91c79c45b09fb7ea165f8e))
* give each sidebar section its own workspace view ([e00eed9](https://github.com/yanghuiwei/Synapse/commit/e00eed996a0865668b494d7bbf303e051548a17c))
* **installer:** brand NSIS installer with Synapse sidebar and header ([11a5fcb](https://github.com/yanghuiwei/Synapse/commit/11a5fcb001c601b5f347d964fa5dff8e63839062))
* **loadtest:** split new-run and history into tabs ([bdf1412](https://github.com/yanghuiwei/Synapse/commit/bdf1412f2291ebbe09823d5943d0d090d278749a))
* P1–P6 全部落地（集合 / 历史 / 变量与 Token / 导入器 / 压测） ([880ae57](https://github.com/yanghuiwei/Synapse/commit/880ae57a1c5e79749b2935826251ea6d41d5057d))
* P7 MCP Server 落地，并修一批交互缺陷 ([4950637](https://github.com/yanghuiwei/Synapse/commit/4950637d2380cc398fb5c5c983e169986ee983a4))
* sidecar 静态链接 VC 运行时，并重写 README 的依赖与打包说明 ([131dea9](https://github.com/yanghuiwei/Synapse/commit/131dea92512296281f01e5b1b0f241f427d85ed6))
* **theme:** add five accent palettes as an orthogonal theme dimension ([e0e6e0f](https://github.com/yanghuiwei/Synapse/commit/e0e6e0f2b01bdd89d29466c3cd17f0ab98bf45ff))
* Token 来源改为内置系统分类页内直配，并入 ApiPost 导入器支持 ([5f89eeb](https://github.com/yanghuiwei/Synapse/commit/5f89eeb8e97ee7645d2c0f426b388c293cfe8fd1))
* Token 源下沉到每个接口（008/009）+ 历史留一份当时的响应（010） ([bc3879b](https://github.com/yanghuiwei/Synapse/commit/bc3879b5e75b397de4ed58068d06a2329ebe9baf))
* Token 缓存与倒计时按「取 token 的配置」共享，而不是按来源记录 ([432c385](https://github.com/yanghuiwei/Synapse/commit/432c385c116758720cbc5b0b680e3bc3e6f8078c))
* **tools:** add a Tools section with a local JWT parser ([354912f](https://github.com/yanghuiwei/Synapse/commit/354912f4d63370409b4ecc48aa9dd6487f6441b6))
* **verify:** 验证 release-please 在死锁解开后会正常开出版本 PR ([#18](https://github.com/yanghuiwei/Synapse/issues/18)) ([e88a286](https://github.com/yanghuiwei/Synapse/commit/e88a286b5a229a4c0fb477a73cde8778f409a200))
* **window:** 自绘标题栏（去系统标题栏 + 整条顶栏可拖 + 窗口按钮） ([de3bf6e](https://github.com/yanghuiwei/Synapse/commit/de3bf6ed7bc36d367b8441488f5ea1628bfa4d7f))
* **window:** 自绘标题栏（去系统标题栏 + 整条顶栏可拖 + 窗口按钮） ([19b43f4](https://github.com/yanghuiwei/Synapse/commit/19b43f4b5fa177ae0e9a57c553646d1b3728229f))
* **workspace:** give history and loadtest their own main views ([243037b](https://github.com/yanghuiwei/Synapse/commit/243037b5693b15f8edb871f0a60216bab927e9f8))
* 侧栏新建收敛为组合按钮，并打通「草稿存进分类」这条断链 ([03f450f](https://github.com/yanghuiwei/Synapse/commit/03f450fc55659e2ae2f8243042afa7c984c0c05f))
* 再次验证 release-please（tag 类型问题） ([5247323](https://github.com/yanghuiwei/Synapse/commit/52473239e695179df93824f768acd1cbeef04f79))
* 压测历史补上「当时那一轮是怎么打的」 ([ec244b4](https://github.com/yanghuiwei/Synapse/commit/ec244b4e7b796e1e4c6c6db4b18c9a52027623fa))
* 取 Token 接口改成自定义下拉 + Ctrl+K 命令面板 ([65c6ea3](https://github.com/yanghuiwei/Synapse/commit/65c6ea312998eeb6d0460d97d791b72649c217e0))
* 取消「授权」概念 —— 凭据就是请求头里的一行（迁移 007） ([eb29991](https://github.com/yanghuiwei/Synapse/commit/eb29991243539f428d1d80ff874fc73da9ae7fc5))
* 应用标识改成 com.reqly.desktop，配套老数据一次性迁移 ([0500fb5](https://github.com/yanghuiwei/Synapse/commit/0500fb523f488acb52f59df484f96dde67f6c8ed))
* 拆 crate 加 CLI 通道，MCP / CLI 进安装包 ([5dbda96](https://github.com/yanghuiwei/Synapse/commit/5dbda96bc736cc2499134f79a656124de4c86fb9))
* 支持 multipart/form-data 请求体与二进制响应流落盘，并收尾 Token 表单 ([54f1b17](https://github.com/yanghuiwei/Synapse/commit/54f1b1713af450249103586dccc747415b72944d))
* 死锁解开后验证 release-please 会正常开出版本 PR ([9abe691](https://github.com/yanghuiwei/Synapse/commit/9abe6917d1d723bcc7a6c20c48a0cae3cd5bf939))
* 验证 autorelease 标签修正后 release-please 恢复 ([48de1f5](https://github.com/yanghuiwei/Synapse/commit/48de1f5b8ee18ac181d0f9e73480e5f2e461176c))
* 验证翻标签后 release-please 能继续开版本 PR ([54fdb70](https://github.com/yanghuiwei/Synapse/commit/54fdb70ca7031e65afe65e752e6e5980861501c8))


### Bug Fixes

* **ci:** correct release-please extra-files schema and tauri-action i… ([0f4dbcf](https://github.com/yanghuiwei/Synapse/commit/0f4dbcf70453148c526a0e1bae6240621b50eb95))
* **ci:** correct release-please extra-files schema and tauri-action input name ([a3147ab](https://github.com/yanghuiwei/Synapse/commit/a3147ab37b107ac4933bbc6af6af1a29ab60e253))
* **ci:** create release-please GitHub Releases as drafts ([31f2753](https://github.com/yanghuiwei/Synapse/commit/31f2753564b89b8580a5d9dd126a5b5ab97408c4))
* **ci:** create release-please GitHub Releases as drafts ([fb72a6f](https://github.com/yanghuiwei/Synapse/commit/fb72a6f79372522ed766bbad29d861892fc83924))
* **ci:** drop stale brand prefix from Release titles ([e547c89](https://github.com/yanghuiwei/Synapse/commit/e547c897a212ab3ae17fb83fc6d1d3f3abfaddfa))
* **ci:** drop stale brand prefix from Release titles ([c024ad1](https://github.com/yanghuiwei/Synapse/commit/c024ad1a08f38da93dbc6f969553a33c2ff38ada))
* **ci:** let publish own the Release object, release-please owns the tag ([5b62ea3](https://github.com/yanghuiwei/Synapse/commit/5b62ea353e44992d8788312d593da14375c4d741))
* **ci:** let publish own the Release object, release-please owns the tag ([2027dbf](https://github.com/yanghuiwei/Synapse/commit/2027dbfe91e710507e0d36dbf9367b9ef98a04da))
* **ci:** release-please 的 token 允许回落到 GITHUB_TOKEN ([480202d](https://github.com/yanghuiwei/Synapse/commit/480202d7ef4404bfeef54f0c3126a400ab59f423))
* **ci:** run: 里不能用 ${{ GITHUB_REPOSITORY }} 表达式 ([0f432a6](https://github.com/yanghuiwei/Synapse/commit/0f432a63fd64b95dce1828a2b141fb6515743fb5))
* **ci:** tag 步骤漏了 VERSION 环境变量（unbound variable） ([f48c550](https://github.com/yanghuiwei/Synapse/commit/f48c550082c4887b2b87f6a25fa7aaf0112171ed))
* **ci:** trigger publish on release-commit push to main, not on tags ([8b09aa3](https://github.com/yanghuiwei/Synapse/commit/8b09aa333b6f58ace27a6e568ea45b758fec3805))
* **ci:** trigger publish on release-commit push to main, not on tags ([ec5895e](https://github.com/yanghuiwei/Synapse/commit/ec5895e98f52b781955e2370ad9b369641238017))
* **ci:** VERSION 是 shell 算的，GitHub 表达式够不着 ([fa3a2e2](https://github.com/yanghuiwei/Synapse/commit/fa3a2e25efb1e8dd055b72c6b39dd54718ef5f34))
* **ci:** Windows 上的测试降级为不阻塞，避免已知 flaky 拖垮 PR ([08b5b56](https://github.com/yanghuiwei/Synapse/commit/08b5b5684e927accad114f885fa28a83b0c6e669))
* **ci:** 修回被 markdown 代码围栏破坏的工作流 YAML ([92e91c8](https://github.com/yanghuiwei/Synapse/commit/92e91c8358b89ea172bc2941a359d7de6e0bc712))
* **ci:** 修回被 markdown 代码围栏破坏的工作流 YAML ([3021694](https://github.com/yanghuiwei/Synapse/commit/30216940bb59bdc6db8a496f710752f091f2eeee))
* **ci:** 修回被 Python f-string 吃掉花括号的 GH_TOKEN ([8f3e467](https://github.com/yanghuiwei/Synapse/commit/8f3e467fcb319963a6a705ed4d1862a4a02cce37))
* **ci:** 修回被压扁的 shell 续行，并把 PR 查找改成 API + 精确比较 ([256d1b5](https://github.com/yanghuiwei/Synapse/commit/256d1b5ffa6b0c18dd291ad096aa313917fdf440))
* **ci:** 删掉 squash 合并留下的重复 target 段 ([ab7c360](https://github.com/yanghuiwei/Synapse/commit/ab7c360e7870406f50333a122d28de9c86a4c4c9))
* **ci:** 加脚本修复 release-please 的 autorelease 标签死锁 ([2f479ad](https://github.com/yanghuiwei/Synapse/commit/2f479ad0bd3257ebd3a4fdb2c72c58b1bebbaef8))
* **ci:** 加脚本修复 release-please 的 autorelease 标签死锁 ([7189748](https://github.com/yanghuiwei/Synapse/commit/7189748e25c8db4bb01b55027706224b5c7070ac))
* **ci:** 发布 tag 必须用轻量 tag，附注 tag 会让 release-please 永远判为未打 tag ([c7359b4](https://github.com/yanghuiwei/Synapse/commit/c7359b4693f4c992a2efbe5676854a82ab29e2d2))
* **ci:** 发布流水线补推 tag，解开 release-please 的闭环死锁 ([623ba1b](https://github.com/yanghuiwei/Synapse/commit/623ba1bb03faefc3a7c2c12372632f76b2c03656))
* **ci:** 发版流程第三步自动化 —— 发布后自动翻 release PR 的标签 ([370be0e](https://github.com/yanghuiwei/Synapse/commit/370be0e872be453640207fdc641dd362ac5a76ff))
* **ci:** 桌面端 check 前先 stage sidecar，修掉 clean clone 上必现的失败 ([6312d32](https://github.com/yanghuiwei/Synapse/commit/6312d32d7546a9a7767ca75d23df844c49337550))
* **ci:** 翻标签那步不能用 gh pr list --search（绿而不动） ([72d2117](https://github.com/yanghuiwei/Synapse/commit/72d211716e7fe936fa950dfd2aabd04cd695e2d0))
* **ci:** 订正 publish.yml 里关于 tag 的错误注释 ([d0f743c](https://github.com/yanghuiwei/Synapse/commit/d0f743cf57e42493951287988ccff170fe00d8b3))
* **ci:** 连注释里的 GitHub 表达式也会被解析 —— 改成纯文字描述 ([d7b0e94](https://github.com/yanghuiwei/Synapse/commit/d7b0e9487df9f55cd91d9c85079a29bb6627af14))
* **ci:** 通用依赖被误塞进 cfg(windows) 段，macOS/Linux 构建全红 ([#16](https://github.com/yanghuiwei/Synapse/issues/16)) ([03a854b](https://github.com/yanghuiwei/Synapse/commit/03a854bdc02cf9fc4da7927243df8d6c89d04f6c))
* **collections:** 搜索时命中的请求不再被所属分类吞掉 ([93cd8cc](https://github.com/yanghuiwei/Synapse/commit/93cd8cce3e38fb6838b488c2df7537af16d4feca))
* **installer:** 缩短 PATH 超限提示，并补一个 PATH 清理脚本（§20.8/§20.9） ([7930996](https://github.com/yanghuiwei/Synapse/commit/793099626a2a79d9bfeab433d94d054e93d4350c))
* **loadtest:** stop restating the run number in the report ([49b6177](https://github.com/yanghuiwei/Synapse/commit/49b6177e2dd70e55561c2cf0250099e402306b6a))
* **response:** keep the pretty/raw toggle usable after switching to raw ([371b70a](https://github.com/yanghuiwei/Synapse/commit/371b70ac74136961e516987c2ceccd8cd4dc8ac0))
* 修复内联 body 被静默丢弃，并补两条 CLI 管理类入口 ([99f361c](https://github.com/yanghuiwei/Synapse/commit/99f361cc37b5bf0fd212c9dfa2a5ca6e28efe908))
* 表格长值改为折行显示，并修 urlencoded 换行分隔、切类型丢内容与 IconButton 图标三处静默失效 ([da10fd9](https://github.com/yanghuiwei/Synapse/commit/da10fd9b293a1c57daa0d4bd26a12427f2ea5965))


### Performance Improvements

* **ui:** 统一状态徽标与提示带，并给浮层加入场动效 ([1535a72](https://github.com/yanghuiwei/Synapse/commit/1535a720ee80b8b36cf9d7a31030a380c9446753))


### Reverts

* 撤掉注定 403 的「同步 release PR 标签」步骤 ([7853826](https://github.com/yanghuiwei/Synapse/commit/7853826d4c955c2eeed20b70656b88de96adb38e))

## [0.7.1](https://github.com/yanghuiwei/Synapse/compare/v0.7.0...v0.7.1) (2026-10-14)


### Bug Fixes

* **ci:** run: 里不能用 ${{ GITHUB_REPOSITORY }} 表达式 ([d4a6571](https://github.com/yanghuiwei/Synapse/commit/d4a657108014a011752b0fbcf814efc262f42591))
* **ci:** tag 步骤漏了 VERSION 环境变量（unbound variable） ([bbdbdc0](https://github.com/yanghuiwei/Synapse/commit/bbdbdc01456deefb30eb72929ddad29cba9714cd))
* **ci:** VERSION 是 shell 算的，GitHub 表达式够不着 ([11f5d0d](https://github.com/yanghuiwei/Synapse/commit/11f5d0d0aab38dcfbeb4a75f0ff143507cf79d0c))
* **ci:** 修回被压扁的 shell 续行，并把 PR 查找改成 API + 精确比较 ([7abe7b6](https://github.com/yanghuiwei/Synapse/commit/7abe7b6483cd526ab3bba30843ac15e72cbad787))
* **ci:** 翻标签那步不能用 gh pr list --search（绿而不动） ([8c696fb](https://github.com/yanghuiwei/Synapse/commit/8c696fbdbda2fdbe975c1c1f0879b2d96af2fc62))
* **ci:** 连注释里的 GitHub 表达式也会被解析 —— 改成纯文字描述 ([6e6d86f](https://github.com/yanghuiwei/Synapse/commit/6e6d86ffa32476f8a6df7a9b4dbc5803b4eabff4))


### Performance Improvements

* **ui:** 统一状态徽标与提示带，并给浮层加入场动效 ([912ede3](https://github.com/yanghuiwei/Synapse/commit/912ede396a35e1fe0c34538c53e666f58285a31a))


### Reverts

* 撤掉注定 403 的「同步 release PR 标签」步骤 ([2c09717](https://github.com/yanghuiwei/Synapse/commit/2c0971759dd3c5c76ec3e68a4dd9a852a72f2423))

## [0.7.0](https://github.com/yanghuiwei/Synapse/compare/v0.6.0...v0.7.0) (2026-10-13)


### Features

* 验证翻标签后 release-please 能继续开版本 PR ([628e659](https://github.com/yanghuiwei/Synapse/commit/628e6592f79ce571ba33188747cf9d332a747e57))


### Bug Fixes

* **ci:** 发版流程第三步自动化 —— 发布后自动翻 release PR 的标签 ([2dd7487](https://github.com/yanghuiwei/Synapse/commit/2dd7487951b3ca76aebc741ea4e11d1dcfa1f158))

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
