; path-hook.nsh 的自测脚本。**不是安装包的一部分**，构建时不会被引用。
;
; 它把同一组宏打到一个**测试用的注册表键**上（见 REQLY_PATH_KEY），
; 于是可以在完全不碰真实 PATH 的前提下验证逻辑，包括那些「只有出事才知道错了」的分支。
;
; 两个坑，先记下来：
;   * 本目录的 .nsh / .nsi 必须存成 **UTF-8 带 BOM**，否则 makensis 按 ACP(GBK) 解码，
;     中文注释会被撕碎并报 "Bad text encoding"。Tauri 生成的 installer.nsi 也是带 BOM 的。
;   * 同一段脚本里插入同一个宏两次会撞标签（标签在 Section / Function 内唯一），
;     所以这里把每种操作包进各自的 Function。
;
; 用法（Windows PowerShell，在 src-tauri/nsis 下）：
;   & "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe" /V2 path-hook.selftest.nsi
;   .\path-hook.selftest.exe /S /MODE=append
; 用例编排与断言在 scripts/nsis-path-hook-test.ps1 里。

!ifndef REQLY_PATH_KEY
  !define REQLY_PATH_KEY "Software\ReqlyPathHookSelfTest"
!endif
; 让 $INSTDIR 确定下来，且刻意带一个空格 —— PATH 里带空格的目录很常见
!define REQLY_SELFTEST_DIR "C:\Program Files\reqly selftest"
!include "path-hook.nsh"

!include FileFunc.nsh

Name "path-hook selftest"
OutFile "path-hook.selftest.exe"
RequestExecutionLevel user
InstallDir "${REQLY_SELFTEST_DIR}"
ShowInstDetails show

Var Mode

Function .onInit
  ${GetOptions} $CMDLINE "/MODE=" $Mode
FunctionEnd

Function DoAppend
  !insertmacro ReqlyPathAppend
FunctionEnd

Function DoRemove
  !insertmacro ReqlyPathRemove
FunctionEnd

Section
  SetRegView 64
  ; 让断言知道宏用的是哪个目录（$INSTDIR 在非安装器里的取值不写死假设）
  WriteRegStr HKCU "${REQLY_PATH_KEY}" "LastInstDir" "$INSTDIR"

  StrCmp $Mode "probe" reqly_selftest_probe
  StrCmp $Mode "append" reqly_selftest_append
  StrCmp $Mode "remove" reqly_selftest_remove
  StrCmp $Mode "both" reqly_selftest_both
  DetailPrint "selftest: 未知 /MODE=$Mode"
  Goto reqly_selftest_end

; 探针：NSIS 的字符串到底能装多长。读的上限与写的上限决定了路径追加的安全边界，
; 所以要用真实环境量一次，而不是照文档写个假设（差一个字符就等于「静默截断」）。
; Probe 由一个 5000 字符的值填进来，远超任何上限，于是读到的长度就是读取上限；
; 再把读到的值自拼三次（源远长于上限），拼完的长度就是写入上限。
reqly_selftest_probe:
  ReadRegStr $0 HKCU "${REQLY_PATH_KEY}" "Probe"
  StrLen $1 $0
  WriteRegStr HKCU "${REQLY_PATH_KEY}" "ProbeReadLen" "$1"
  StrCpy $9 "$0$0$0"
  StrLen $2 $9
  WriteRegStr HKCU "${REQLY_PATH_KEY}" "ProbeWriteLen" "$2"
  Goto reqly_selftest_end

reqly_selftest_append:
  Call DoAppend
  Goto reqly_selftest_end

reqly_selftest_remove:
  Call DoRemove
  Goto reqly_selftest_end

reqly_selftest_both:
  Call DoAppend
  Call DoRemove
  Goto reqly_selftest_end

reqly_selftest_end:
SectionEnd
