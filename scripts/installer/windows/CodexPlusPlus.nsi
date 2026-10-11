Unicode true
!include "MUI2.nsh"
!include "StrFunc.nsh"
${Using:StrFunc} StrStr
${Using:StrFunc} UnStrStr

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!define ROOT "..\..\.."

Name "Codex++"
OutFile "${ROOT}\dist\windows\CodexPlusPlus-${VERSION}-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Codex++"
; 沿用旧安装目录登记，覆盖升级继续安装到原目录。
InstallDirRegKey HKCU "Software\Codex++" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Var ExistingInstall
Var HadDesktopEntry

!define MUI_ICON "${ROOT}\apps\codex-plus-manager\src-tauri\icons\icon.ico"
!define MUI_UNICON "${ROOT}\apps\codex-plus-manager\src-tauri\icons\icon.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; 主程序退出时负责恢复系统浏览器及清理后台服务，安装器不能强制结束进程。
; 每 500ms 检查一次；10 秒仍未退出时提示从托盘退出，再重试或取消安装/卸载。
; 确认进程退出后再补 500ms 文件锁释放缓冲，避免 File/Delete 撞上残留锁（issue #2362）。
; 判据用 tasklist 输出 + 字符串包含，不用其退出码（实测 Win11 无匹配仍返回 0），
; 也不用 cmd 管道 find（PATH 里的 Unix find.exe 会劫持、多层引号易变形）。
!macro WAIT_FOR_PROCESS_EXIT UNFUNC UNSTR
  Function ${UNFUNC}
    Pop $1
    StrCpy $2 0
  wait_loop:
    nsExec::ExecToStack '$SYSDIR\tasklist.exe /FI "IMAGENAME eq $1" /NH'
    Pop $0
    Pop $9
    !if "${UNSTR}" == ""
      ${StrStr} $8 "$9" "$1"
    !else
      ${UnStrStr} $8 "$9" "$1"
    !endif
    StrCmp $8 "" wait_done 0
    IntOp $2 $2 + 1
    IntCmp $2 20 wait_prompt 0 wait_prompt
    Sleep 500
    Goto wait_loop
  wait_prompt:
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "请从系统托盘退出 Codex++，等待后台服务清理完成后重试。" IDRETRY wait_retry IDCANCEL wait_cancel
  wait_retry:
    StrCpy $2 0
    Goto wait_loop
  wait_cancel:
    Abort
  wait_done:
    Sleep 500
  FunctionEnd
!macroend
!insertmacro WAIT_FOR_PROCESS_EXIT WaitForProcessExit ""
!insertmacro WAIT_FOR_PROCESS_EXIT un.WaitForProcessExit un.

Section "Install"
  SetOutPath "$INSTDIR"
  ; 与 Rust 安装入口共用卸载登记，必须在本轮写登记之前判断是否覆盖升级。
  ReadRegStr $ExistingInstall HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "InstallLocation"
  StrCmp $ExistingInstall "" 0 existing_install_checked
  ReadRegStr $ExistingInstall HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++" "InstallLocation"
  existing_install_checked:

  Push "codex-plus-plus.exe"
  Call WaitForProcessExit
  Push "codex-plus-plus-manager.exe"
  Call WaitForProcessExit

  ; 双程序版本升级时只保留统一应用；清理失败则停止，避免残留旧管理工具。
  Delete "$INSTDIR\codex-plus-plus-manager.exe"
  IfFileExists "$INSTDIR\codex-plus-plus-manager.exe" 0 legacy_binary_removed
    MessageBox MB_OK|MB_ICONSTOP "旧管理工具仍被占用。请退出 Codex++ 后重新安装。"
    Abort
  legacy_binary_removed:
  File "${ROOT}\dist\windows\app\codex-plus-plus.exe"

  ; 只迁移确实存在的桌面入口，用户删掉的入口在升级时仍不补建（issue #2376）。
  StrCpy $HadDesktopEntry 0
  IfFileExists "$DESKTOP\Codex++.lnk" desktop_entry_found 0
  IfFileExists "$DESKTOP\Codex++ 管理工具.lnk" desktop_entry_found 0
  IfFileExists "$DESKTOP\Codex++ 绠＄悊宸ュ叿.lnk" desktop_entry_found desktop_entry_checked
  desktop_entry_found:
  StrCpy $HadDesktopEntry 1
  desktop_entry_checked:
  Delete "$DESKTOP\Codex++.lnk"
  Delete "$DESKTOP\Codex++ 管理工具.lnk"
  Delete "$DESKTOP\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 管理工具.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\卸载 Codex++.lnk"

  StrCmp $ExistingInstall "" desktop_entry_create 0
  StrCmp $HadDesktopEntry 1 desktop_entry_create desktop_entry_done
  desktop_entry_create:
  CreateShortcut "$DESKTOP\Codex++.lnk" "$INSTDIR\codex-plus-plus.exe" "" "$INSTDIR\codex-plus-plus.exe"
  desktop_entry_done:
  CreateDirectory "$SMPROGRAMS\Codex++"
  CreateShortcut "$SMPROGRAMS\Codex++\Codex++.lnk" "$INSTDIR\codex-plus-plus.exe" "" "$INSTDIR\codex-plus-plus.exe"
  CreateShortcut "$SMPROGRAMS\Codex++\卸载 Codex++.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\codex-plus-plus.exe"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Codex++" "InstallDir" "$INSTDIR"
  ; 卸载项键名统一到运行时的正式键 CodexPlusPlus（issue #2339）。
  ; 运行时 install/windows.rs 把 Uninstall\CodexPlusPlus 当正式键、Uninstall\Codex++
  ; 当 legacy；这里若继续写 Codex++，经安装器装过又被管理工具碰过的机器会留下两条
  ; 卸载项，而运行时装过、之后走 NSIS 卸载的机器永远删不掉 CodexPlusPlus 那条。
  ; 写之前先清掉 legacy 键：存量用户靠这一步收尸，别删。
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "DisplayName" "Codex++"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "Publisher" "BigPizzaV3"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "DisplayIcon" "$INSTDIR\codex-plus-plus.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "UninstallString" "$INSTDIR\uninstall.exe"

  ; 注册 codexplusplus:// 与 dreamskin:// URL 协议（issue #2354）。
  ; 键名与值同 codex-plus-core 的 install::windows::register_url_protocol 保持一致，
  ; 应用内"安装入口/修复快捷方式"会以相同键幂等覆盖，两边互不冲突。
  WriteRegStr HKCU "Software\Classes\codexplusplus" "" "URL:Codex++ Import Protocol"
  WriteRegStr HKCU "Software\Classes\codexplusplus" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\codexplusplus\shell\open\command" "" '"$INSTDIR\codex-plus-plus.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\dreamskin" "" "URL:DreamSkin Community Theme Protocol"
  WriteRegStr HKCU "Software\Classes\dreamskin" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\dreamskin\shell\open\command" "" '"$INSTDIR\codex-plus-plus.exe" "%1"'

  ; 仅迁移已经存在的自有开机项；不补建未启用的项，也不改 watcher.disabled。
  ; 新参数只打开界面，并由主程序检查禁用标记，旧 --debug-port 参数不再承担启动职责。
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CodexPlusPlusWatcher"
  StrCmp $0 "" watcher_run_done 0
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CodexPlusPlusWatcher" '"$INSTDIR\codex-plus-plus.exe" --autostart'
  watcher_run_done:
  IfFileExists "$SMSTARTUP\CodexPlusPlusWatcher.lnk" 0 watcher_shortcut_done
  CreateShortcut "$SMSTARTUP\CodexPlusPlusWatcher.lnk" "$INSTDIR\codex-plus-plus.exe" "--autostart" "$INSTDIR\codex-plus-plus.exe"
  watcher_shortcut_done:
SectionEnd

Section "Uninstall"
  Push "codex-plus-plus.exe"
  Call un.WaitForProcessExit
  Push "codex-plus-plus-manager.exe"
  Call un.WaitForProcessExit

  Delete "$DESKTOP\Codex++.lnk"
  Delete "$DESKTOP\Codex++ 管理工具.lnk"
  Delete "$DESKTOP\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++.lnk"
  Delete "$SMPROGRAMS\Codex++\卸载 Codex++.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 管理工具.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\卸载 Codex++.lnk"
  RMDir "$SMPROGRAMS\Codex++"

  Delete "$INSTDIR\codex-plus-plus.exe"
  Delete "$INSTDIR\codex-plus-plus-manager.exe"

  ; NSIS 的 Delete 遇到被占用文件会静默失败，用户会误以为卸载干净。
  ; 残留检测提示用户先从托盘退出，避免“卸载后重装”继续失败（issue #2362）。
  IfFileExists "$INSTDIR\codex-plus-plus.exe" 0 +2
    MessageBox MB_OK|MB_ICONEXCLAMATION "Codex++ 仍被占用，未能删除。请从系统托盘退出 Codex++（或结束该进程）后重新卸载或安装。"

  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; 正式键 + legacy 键都删（issue #2339）：正式键是运行时写入的 CodexPlusPlus，
  ; legacy 是历史安装器和旧版运行时的 Codex++，存量机器上可能两条并存。
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++"
  DeleteRegKey HKCU "Software\Codex++"

  ; 清理安装器注册的 URL 协议（与 install::windows::uninstall_shortcuts 行为一致）。
  ; 仅删除本产品自有键，不影响用户其他协议注册。
  DeleteRegKey HKCU "Software\Classes\codexplusplus"
  DeleteRegKey HKCU "Software\Classes\dreamskin"
SectionEnd
