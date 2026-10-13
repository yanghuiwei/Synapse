//! 去掉系统标题栏（自绘标题栏）。
//!
//! ## 背景：为什么不用 `Window::set_decorations`
//!
//! `tauri.conf.json` 里的 `"decorations": false` 与运行时的
//! `window.set_decorations(false)` **两条路都试过，都没生效**：
//!
//! - 配置：重新编译后起窗口，`GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_CAPTION`
//!   仍为真，客户区 800x600 而外框 813x608 —— 多出 13px 边框 + 8px 标题栏占位。
//! - 运行时：`setup` 里调 `set_decorations(false)` 返回 `Ok(())`、无任何报错，
//!   **样式位照样没变**。
//!
//! 沿 tauri-2.11.5 → tauri-runtime-wry-2.11.4 → tao-0.35.3 查过：
//! `tao::platform_impl::windows::window::set_decorations` 只是
//! `execute_in_thread(...)` 投递一个「改窗口标志位」的闭包，
//! 而 Windows 上 `WS_CAPTION` 决定的是**非客户区的存在**，
//! 光改这一位不足以去掉标题栏。
//!
//! ⚠️ 这个失败模式极其隐蔽：**界面上看不出区别** —— 自绘顶栏里也有个
//! "Synapse"，截图一眼扫过去是「正常的」。只有读 Win32 样式位才发现白做。
//!
//! ## 做法
//!
//! 直接调 Win32：清掉 `WS_CAPTION`（**保留 `WS_THICKFRAME`**，否则边缘拖不动），
//! 重算窗口尺寸让客户区填满原来的外框（否则右侧+底部各留 13x8 的空白），
//! 最后 `SWP_FRAMECHANGED` 让系统重算非客户区与拖拽边界。
//!
//! **刻意保留** `WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`：它们让
//! Alt+F4、Win+←/→ 贴边、任务栏右键菜单这些**系统手势继续有效**。
//! 全清掉的话窗口会变成一个「什么都不能干」的框。
//!
//!
//! ## capability 权限：自绘标题栏必须在 `capabilities/default.json` 里显式授权
//!
//! 说明放在这儿而不是 `default.json` 旁边 —— 后者是 JSON，**没有注释语法**
//! （曾经在 `permissions` 数组里塞 `"//": "..."` 这种键，编译期直接报
//! `JSONDecodeError`，白改一轮）。
//!
//! `default.json` 里本来就有 `core:default`，它**间接包含**
//! `core:window:default`。但那份清单的 28 项里**全是只读状态**
//! （`is-maximized` / `inner-size` / `title` / `is-focused` …），
//! **一个「动窗口」的都没有** —— 最小化 / 最大化切换 / 关闭 / 拖拽全被 ACL 拦下。
//!
//! 表现极具迷惑性：**按钮点了没反应、顶栏拖不动，而前端连报错都收不到**
//! （拒绝发生在 Rust 侧 ACL 层，JS 那边只是 `invoke` 不成功、
//! promise 静默不 resolve）。界面上看就是「按钮画出来了但没用」——
//! 与本轮 P0 修掉的 Token 计数假按钮是同一类错误。
//!
//! | 权限 | 谁在用 |
//! |---|---|
//! | `core:window:allow-minimize` | 最小化按钮 |
//! | `core:window:allow-toggle-maximize` | 最大化 / 还原按钮（**用 `toggle` 不是 `maximize`** —— 要能还原就必须用切换版） |
//! | `core:window:allow-close` | 关闭按钮 |
//! | `core:window:allow-start-dragging` | `data-tauri-drag-region`。Tauri 注入的 `drag.js` 在 mousedown 时 `invoke('plugin:window|start_dragging')` |
//!
//! **已经含、不要重复加**：`core:window:default` 里已有 `allow-is-maximized`
//! （最大化态跟踪靠它）与 `allow-internal-toggle-maximize`
//! （双击标题栏最大化靠它，见 `drag.js` 里 `e.detail === 2` 那条）。
//!
//! ### 查「到底缺哪个权限」的权威来源
//!
//! **`src-tauri/gen/schemas/acl-manifests.json`** —— Tauri 构建时自己生成的
//! 权限清单，比翻源码或猜都可靠：
//!
//! ```python
//! d = json.load(open("gen/schemas/acl-manifests.json"))
//! have = set(d["core:window"]["default_permission"]["permissions"])
//! # 再拿需要的列表去比 have
//! ```
//!
//! ⚠️ `default_permission` 在**每个插件自己的节点下**（`d["core:window"]`），
//! 而 `d["core"]["default_permission"]` 是 `core:default` 的展开
//! （里面只有 `core:window:default` 这种集合名，不是具体权限）——
//! **查错节点会得出「一个权限都没有」的错误结论**。
//!
//! 代价：**丢掉系统的窗口阴影与圆角**（无边框窗口在 Windows 上本来就没有）。
//! 要补阴影得靠 `window-vibrancy` 之类的插件，那是另一轮的事。

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WINDOW_LONG_PTR_INDEX,
    WS_CAPTION, WS_THICKFRAME,
};

/// `GWL_STYLE`（-16）。
///
/// ⚠️ `windows` 0.61 **没有导出 `GWLP_STYLE` 常量**（只有
/// `WINDOW_LONG_PTR_INDEX(pub i32)` 这个元组结构），所以按值构造。
/// 别去找那个常量 —— `grep GWLP_STYLE` 在 0.61 的源码里是零命中。
const GWL_STYLE: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-16);

/// 去掉系统标题栏。返回**复核后**的结果：`true` = 样式位确认已清掉 `WS_CAPTION`。
///
/// 幂等：已经是无边框时直接返回 `true`，不再重算尺寸（重复重算会让窗口「抖」一下）。
///
/// 返回值是**读回样式位复核**的结果而不是「调用成功」——
/// 这一整个模块存在的理由就是「返回成功但没生效」，所以判据必须落在
/// 可独立读取的状态上，不能落在返回值上。
pub fn strip_decorations(hwnd: HWND) -> bool {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if style & WS_CAPTION.0 as isize == 0 {
            return true;
        }

        // ⚠️ **只清 `WS_CAPTION`，保留 `WS_THICKFRAME`**。
        // 一开始两个一起清了，结果窗口边缘拖不动 —— `WS_THICKFRAME` 正是
        // Windows 「边缘可拖拽调整大小」的能力来源，去掉它窗口就成了固定尺寸的框。
        // `WS_THICKFRAME` 不带 `WS_CAPTION` 时只留一条极细的 resize 热区（不可见），
        // 这正是无边框窗口要的行为。
        //
        // 同样保留 `WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`：
        // Alt+F4、Win+←/→ 贴边、任务栏右键菜单都靠它们。
        let new_style = style & !WS_CAPTION.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);

        // ⚠️ 顺序很要紧：**先刷新边框，再量尺寸**。
        // `SetWindowLongPtrW` 只是记下新样式，系统要等到 `SWP_FRAMECHANGED`
        // 才重算边框度量。一开始这里是「改样式 → 量 → 改尺寸 → FRAMECHANGED」，
        // 结果量的时候用的还是**旧边框**。
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .expect("去掉标题栏后刷新边框失败");

        // 把**标题栏那一份**占位还给客户区。
        //
        // ⚠️ 这里曾把判据写成「客户区 == 外框」，**那是错的**，白改了一轮：
        // `WS_THICKFRAME` 在 Windows 上**必然**带来一条 resize border
        // （实测 1080p 上是 13px 宽 / 8px 高），那是它自带的、不可见、
        // 且**必须保留**（去掉它窗口边缘就拖不动了）。
        // 所以「差值不为 0」本身不是 bug —— 要还原的只有**标题栏**那 8px 上下。
        //
        // 正确判据：**高度方向**的差值（标题栏是上下加的）应该只剩 resize
        // border 的那一半；横向那 13px 是左右边框，属正常。
        // 简化成可靠做法：横向按 resize border 的一半补，纵向补满。
        let mut cr = RECT::default();
        let mut wr = RECT::default();
        if GetClientRect(hwnd, &mut cr).is_err() || GetWindowRect(hwnd, &mut wr).is_err() {
            return false;
        }
        let client_w = cr.right - cr.left;
        let client_h = cr.bottom - cr.top;
        let outer_w = wr.right - wr.left;
        let outer_h = wr.bottom - wr.top;

        if client_w != outer_w || client_h != outer_h {
            // 只补一半：resize border 是左右/上下对称的，补满会把窗口撑大
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                outer_w - (outer_w - client_w) / 2,
                outer_h - (outer_h - client_h) / 2,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .expect("去掉标题栏后调整窗口尺寸失败");
        }

        // 复核：读回**样式位**，而不是相信上面几步没报错。
        //
        // ⚠️ 判据只有「`WS_CAPTION` 已清 + `WS_THICKFRAME` 仍在」两项。
        // **不要**加「客户区 == 外框」那种几何判据 —— `WS_THICKFRAME` 天生
        // 带一条 resize border，差值不为 0 是正常的（我在这上面白绕了一轮）。
        // 几何量只能用来**调试**，不能用来判定成败。
        let style_after = GetWindowLongPtrW(hwnd, GWL_STYLE);
        style_after & WS_CAPTION.0 as isize == 0 && style_after & WS_THICKFRAME.0 as isize != 0
    }
}
