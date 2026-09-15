; reqly 的 NSIS 安装钩子 —— 把安装目录加进当前用户的 PATH，让 `reqly-cli` 在任意终端里直接敲得到。
;
; 为什么需要它：MCP 由客户端按绝对路径 spawn（src/commands/mcp.rs::mcp_command_path），
; 全程不碰 PATH；而 CLI 是给人敲的，不在 PATH 就等于「装了但敲不到」，
; 它那一半价值（批处理 / 管道 / 大响应落文件）也就用不上了。
;
; ---------------------------------------------------------------------------
; 长度边界：以下数字**实测过**（NSIS 3.11 + makensis/nsis 3.11，见自测脚本），
; 不是照文档抄的 —— 差一个字符就是「静默毁掉用户的 PATH」。
;
;   * 值长度 ≤ 1023 字符 → ReadRegStr 完整读回（1023 也完整）。
;   * 值长度 ≥ 1024 字符 → ReadRegStr **读失败，返回空串**（不是截断！）。
;   * StrCpy 的结果最多 1023 字符，超出**静默截断**。
;
; 第二条最要命：一条 ≥1024 字符的 PATH（现在很容易达到）读回来跟「用户压根没设 PATH」
; 长得一模一样。要是照这个空串去追加，就会把用户整条 PATH 替换成我们一个目录 ——
; 别人的命令集体失踪，安装过程一声不吭。所以「读回空串」必须再问一次
; 「这个值到底在不在」，见 ReqlyPathValueExists。
;
; 代价：一个「存在但为空」的 Path 也落进同一类，于是我们也不写。这是刻意的取舍 ——
; 核心 NSIS 指令分不出「0 个字符」与「1024 个字符读失败」，而猜错的方向是毁掉整条 PATH。
; 少一条 PATH 记录 vs. 抹掉用户所有命令，代价不对称。
;
; ---------------------------------------------------------------------------
; 四条会静默出错的点，都已挡掉：
;   1. 不无条件追加 —— 已包含就不写。PATH 是用户的，重复项会越滚越长。
;   2. 读回空串要分情况 —— 见上。
;   3. 新值长度先算过 —— 超过 1023 就不写，宁可这次没配上，也不写半截。
;   4. 改完广播 WM_SETTINGCHANGE —— 否则已经开着的终端拿不到新 PATH，
;      用户会以为「装了没生效」。
;
; 只写 HKCU：安装模式是 currentUser（src-tauri/tauri.conf.json），不需要管理员权限，
; 也就不该碰机器级的 PATH。卸载时反向摘掉，免得在注册表里留垃圾。
;
; ---------------------------------------------------------------------------
; ⚠️ 两个维护约束：
;   * 本文件必须存成 **UTF-8 带 BOM**。makensis 拿不到 BOM 就按 ACP(GBK) 解码，
;     中文会被撕碎，报的却是 "Bad text encoding" —— 看着像语法问题，其实是编码问题。
;   * 宏里的标签在 Section / Function 内唯一。同一个 Section 里**别把两个钩子都插进去**，
;     会撞标签（Tauri 模板里装/卸是两个 Section，天然分开）。
;
; 自测见同目录的 path-hook.selftest.nsi（配合 scripts/nsis-path-hook-test.ps1）。

; 留了 !ifndef 口子：自测脚本要把它们指向一个测试用的注册表键，
; 这样验证逻辑不必碰真实的 PATH。
!ifndef REQLY_PATH_KEY
  !define REQLY_PATH_KEY "Environment"
!endif
!ifndef REQLY_PATH_VALUE
  !define REQLY_PATH_VALUE "Path"
!endif
!ifndef HWND_BROADCAST
  !define HWND_BROADCAST 0xffff
!endif
!ifndef WM_SETTINGCHANGE
  !define WM_SETTINGCHANGE 0x001a
!endif

; 写入上限：StrCpy 到 1023 个字符封顶，结果长度 >= 1024 一律不写。
!define REQLY_PATH_MAX 1024

; 广播环境变量变更，让已经开着的资源管理器刷新缓存。
; 用 SendMessageTimeout 而不是 SendMessage：广播不能把安装卡在一个不响应的窗口上。
; 2 = SMTO_ABORTIFHUNG，5000ms 超时。
; 这里刻意不引 EnVar 之类的外部插件 —— 少一个构建期依赖，少一处静默失效的来源。
!macro ReqlyBroadcastEnvChange
  System::Call 'USER32::SendMessageTimeoutW(p ${HWND_BROADCAST}, i ${WM_SETTINGCHANGE}, p 0, w "Environment", i 2, i 5000, *i .r7)'
!macroend

; 判断目标值**本身**在不在（只看名字，不看内容）。出口：$R6 = "1" 在 / "0" 不在。
; 只在「读回来是空串」时用得上，用来区分「用户没设过」与「值太长读不回来」。
!macro ReqlyPathValueExists
  StrCpy $R6 "0"
  StrCpy $R1 0
reqly_val_exists_loop:
  ClearErrors
  EnumRegValue $R2 HKCU "${REQLY_PATH_KEY}" $R1
  IfErrors reqly_val_exists_end
  StrCmp $R2 "${REQLY_PATH_VALUE}" 0 reqly_val_exists_next
    StrCpy $R6 "1"
    Goto reqly_val_exists_end
reqly_val_exists_next:
  IntOp $R1 $R1 + 1
  ; 64 个值足够 HKCU\Environment 用了；这个上限只是防止枚举在某些环境下不报错而空转
  IntCmp $R1 64 reqly_val_exists_end reqly_val_exists_loop reqly_val_exists_end
reqly_val_exists_end:
!macroend

; 追加 $INSTDIR 到用户 PATH 末尾。已包含 / 太长都不算失败 —— 配不上 PATH 不该让安装失败。
!macro ReqlyPathAppend
  ReadRegStr $0 HKCU "${REQLY_PATH_KEY}" "${REQLY_PATH_VALUE}"
  StrLen $1 $0
  StrCmp $1 "0" 0 reqly_path_have_value
  ; 空串：可能是真没设，也可能是长到读不回来。后者绝不能当成「没设」。
  !insertmacro ReqlyPathValueExists
  StrCmp $R6 "1" reqly_path_too_long
reqly_path_have_value:
  ; 先把结果长度算出来：$0 + ';' + $INSTDIR
  StrLen $5 "$INSTDIR"
  IntOp $5 $5 + $1
  IntOp $5 $5 + 1
  IntCmp $5 ${REQLY_PATH_MAX} reqly_path_too_long reqly_path_scan_start reqly_path_too_long

reqly_path_scan_start:
  StrCpy $R0 "0"
  StrCpy $2 "$0"
; 逐条目扫，而不是在整串里找子串：`C:\reqly` 与 `C:\reqly2` 是两回事。
reqly_path_scan:
  StrCmp $2 "" reqly_path_scan_done
  StrCpy $1 0
reqly_path_find_sep:
  StrCpy $4 "$2" 1 $1
  StrCmp $4 "" reqly_path_last_entry
  StrCmp $4 ";" reqly_path_entry_ready
  IntOp $1 $1 + 1
  Goto reqly_path_find_sep
reqly_path_entry_ready:
  StrCpy $3 "$2" $1
  IntOp $1 $1 + 1
  StrCpy $2 "$2" "" $1
  Goto reqly_path_entry_cmp
reqly_path_last_entry:
  StrCpy $3 "$2"
  StrCpy $2 ""
reqly_path_entry_cmp:
  StrCmp $3 "" reqly_path_scan
  ; 去引号
  StrCpy $4 "$3" 1
  StrCmp $4 '"' 0 reqly_path_no_lead_quote
    StrCpy $4 "$3" "" 1
    StrCpy $3 "$4"
  reqly_path_no_lead_quote:
  StrCmp $3 "" reqly_path_scan
  StrCpy $4 "$3" 1 -1
  StrCmp $4 '"' 0 reqly_path_no_tail_quote
    StrCpy $4 "$3" -1
    StrCpy $3 "$4"
  reqly_path_no_tail_quote:
  StrCpy $4 "$3"
  StrCpy $1 "$4" 1 -1
  ; 末尾的反斜杠不参与比较：`C:\app` 与 `C:\app\` 指同一个目录
  StrCmp $1 "\" 0 reqly_path_cmp_entry
    StrCpy $4 "$4" -1
  reqly_path_cmp_entry:
  ; StrCmp 本身不区分大小写 —— 正合 PATH 的语义，Windows 比路径不看大小写
  StrCmp $4 "$INSTDIR" 0 reqly_path_scan
    StrCpy $R0 "1"
  Goto reqly_path_scan_done

reqly_path_scan_done:
  StrCmp $R0 "1" reqly_path_already
; 去掉尾部的 ';'，否则会拼出 `...;;C:\...`
reqly_path_trim:
  StrCmp $0 "" reqly_path_new_first
  StrCpy $1 "$0" 1 -1
  StrCmp $1 ";" 0 reqly_path_new_join
    StrCpy $0 "$0" -1
    Goto reqly_path_trim
reqly_path_new_join:
  StrCpy $9 "$0;$INSTDIR"
  Goto reqly_path_write
reqly_path_new_first:
  StrCpy $9 "$INSTDIR"
reqly_path_write:
  WriteRegExpandStr HKCU "${REQLY_PATH_KEY}" "${REQLY_PATH_VALUE}" "$9"
  !insertmacro ReqlyBroadcastEnvChange
  DetailPrint "reqly: 已把 $INSTDIR 加入用户 PATH（新开的终端生效）"
  Goto reqly_path_end

reqly_path_already:
  DetailPrint "reqly: 用户 PATH 已包含 $INSTDIR，未改动"
  Goto reqly_path_end

reqly_path_too_long:
  ; 宁可不动，也不写半截。用户拿到的是「没配上自动补全」，不是「一堆命令突然找不到」。
  DetailPrint "reqly: 用户 PATH 过长或读不回来，未自动写入 —— 请手工把 $INSTDIR 加进 PATH"
  IfSilent reqly_path_end
  MessageBox MB_OK|MB_ICONINFORMATION "reqly 没有修改你的 PATH。$\r$\n$\r$\n它太长（或读不回来），自动追加有可能把它整条弄坏 —— 那会连带影响你其它命令。$\r$\n想让命令行直接敲 reqly-cli 的话，请手工把安装目录加进 PATH。"
reqly_path_end:
!macroend

; 卸载时摘掉 $INSTDIR。同样：读不回来就宁可留着这条，也不写回去。
!macro ReqlyPathRemove
  ReadRegStr $0 HKCU "${REQLY_PATH_KEY}" "${REQLY_PATH_VALUE}"
  StrLen $1 $0
  StrCmp $1 "0" 0 reqly_unpath_have_value
  !insertmacro ReqlyPathValueExists
  StrCmp $R6 "1" reqly_unpath_too_long
reqly_unpath_have_value:
  StrCpy $2 "$0"
  StrCpy $9 ""
reqly_unpath_scan:
  StrCmp $2 "" reqly_unpath_write
  StrCpy $1 0
reqly_unpath_find_sep:
  StrCpy $4 "$2" 1 $1
  StrCmp $4 "" reqly_unpath_last
  StrCmp $4 ";" reqly_unpath_ready
  IntOp $1 $1 + 1
  Goto reqly_unpath_find_sep
reqly_unpath_ready:
  StrCpy $3 "$2" $1
  IntOp $1 $1 + 1
  StrCpy $2 "$2" "" $1
  Goto reqly_unpath_cmp
reqly_unpath_last:
  StrCpy $3 "$2"
  StrCpy $2 ""
reqly_unpath_cmp:
  StrCmp $3 "" reqly_unpath_scan
  ; 比较用副本，写回去的仍是原样的 $3 —— 别顺手改掉别人条目的写法
  StrCpy $4 "$3"
  StrCpy $1 "$4" 1 -1
  StrCmp $1 "\" 0 reqly_unpath_cmp_entry
    StrCpy $4 "$4" -1
  reqly_unpath_cmp_entry:
  StrCmp $4 "$INSTDIR" reqly_unpath_scan
  StrCmp $9 "" reqly_unpath_keep_first
    StrCpy $9 "$9;$3"
    Goto reqly_unpath_scan
reqly_unpath_keep_first:
  StrCpy $9 "$3"
  Goto reqly_unpath_scan

reqly_unpath_write:
  StrCmp $9 "$0" reqly_unpath_already
  WriteRegExpandStr HKCU "${REQLY_PATH_KEY}" "${REQLY_PATH_VALUE}" "$9"
  !insertmacro ReqlyBroadcastEnvChange
  DetailPrint "reqly: 已从用户 PATH 摘掉 $INSTDIR"
  Goto reqly_unpath_end

reqly_unpath_already:
  DetailPrint "reqly: 用户 PATH 里没有 $INSTDIR，未改动"
  Goto reqly_unpath_end

reqly_unpath_too_long:
  DetailPrint "reqly: 用户 PATH 过长或读不回来，未自动清理 —— 可以手工摘掉 $INSTDIR"
reqly_unpath_end:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro ReqlyPathAppend
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro ReqlyPathRemove
!macroend
