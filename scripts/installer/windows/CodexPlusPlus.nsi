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
InstallDirRegKey HKCU "Software\Codex++" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Var ExistingInstall

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

; taskkill 只发出终止指令，且进程从进程表消失也不代表镜像文件锁已释放
; （进程退出清理、WebView2 子进程树、杀软扫描等会延迟数秒）。
; 轮询等待进程真正退出：500ms 一次、最多 10 秒；等到过进程再补 500ms 锁释放缓冲，
; 避免紧随其后的 File/Delete 撞上残留文件锁（issue #2362）。
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
    IntCmp $2 20 wait_done 0 wait_done
    Sleep 500
    Goto wait_loop
  wait_done:
    IntCmp $2 0 lock_buffer_done 0 lock_buffer_done
    Sleep 500
  lock_buffer_done:
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

  nsExec::ExecToLog 'taskkill /IM codex-plus-plus.exe /F'
  Pop $0
  nsExec::ExecToLog 'taskkill /IM codex-plus-plus-manager.exe /F'
  Pop $0
  Push "codex-plus-plus.exe"
  Call WaitForProcessExit
  Push "codex-plus-plus-manager.exe"
  Call WaitForProcessExit

  File "${ROOT}\dist\windows\app\codex-plus-plus.exe"
  File "${ROOT}\dist\windows\app\codex-plus-plus-manager.exe"

  Delete "$DESKTOP\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 绠＄悊宸ュ叿.lnk"

  ; 仅首次安装补桌面图标（issue #2376）。覆盖升级保留现状，用户删掉的
  ; 图标不能再次添加；首次安装已有的图标也保留。开始菜单仍照常补建。
  StrCmp $ExistingInstall "" 0 desktop_silent_done
  IfFileExists "$DESKTOP\Codex++.lnk" desktop_silent_done 0
  CreateShortcut "$DESKTOP\Codex++.lnk" "$INSTDIR\codex-plus-plus.exe" "" "$INSTDIR\codex-plus-plus.exe"
  desktop_silent_done:
  StrCmp $ExistingInstall "" 0 desktop_manager_done
  IfFileExists "$DESKTOP\Codex++ 管理工具.lnk" desktop_manager_done 0
  CreateShortcut "$DESKTOP\Codex++ 管理工具.lnk" "$INSTDIR\codex-plus-plus-manager.exe" "" "$INSTDIR\codex-plus-plus-manager.exe"
  desktop_manager_done:
  CreateDirectory "$SMPROGRAMS\Codex++"
  CreateShortcut "$SMPROGRAMS\Codex++\Codex++.lnk" "$INSTDIR\codex-plus-plus.exe" "" "$INSTDIR\codex-plus-plus.exe"
  CreateShortcut "$SMPROGRAMS\Codex++\Codex++ 管理工具.lnk" "$INSTDIR\codex-plus-plus-manager.exe" "" "$INSTDIR\codex-plus-plus-manager.exe"
  CreateShortcut "$SMPROGRAMS\Codex++\卸载 Codex++.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\codex-plus-plus-manager.exe"

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
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "DisplayIcon" "$INSTDIR\codex-plus-plus-manager.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus" "UninstallString" "$INSTDIR\uninstall.exe"

  ; 注册 codexplusplus:// 与 dreamskin:// URL 协议（issue #2354）。
  ; 键名与值同 codex-plus-core 的 install::windows::register_url_protocol 保持一致，
  ; 管理工具内"安装入口/修复快捷方式"会以相同键幂等覆盖，两边互不冲突。
  WriteRegStr HKCU "Software\Classes\codexplusplus" "" "URL:Codex++ Import Protocol"
  WriteRegStr HKCU "Software\Classes\codexplusplus" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\codexplusplus\shell\open\command" "" '"$INSTDIR\codex-plus-plus-manager.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\dreamskin" "" "URL:DreamSkin Community Theme Protocol"
  WriteRegStr HKCU "Software\Classes\dreamskin" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\dreamskin\shell\open\command" "" '"$INSTDIR\codex-plus-plus-manager.exe" "%1"'
SectionEnd

Section "Uninstall"
  nsExec::ExecToLog 'taskkill /IM codex-plus-plus.exe /F'
  Pop $0
  nsExec::ExecToLog 'taskkill /IM codex-plus-plus-manager.exe /F'
  Pop $0
  Push "codex-plus-plus.exe"
  Call un.WaitForProcessExit
  Push "codex-plus-plus-manager.exe"
  Call un.WaitForProcessExit

  Delete "$DESKTOP\Codex++.lnk"
  Delete "$DESKTOP\Codex++ 管理工具.lnk"
  Delete "$DESKTOP\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 管理工具.lnk"
  Delete "$SMPROGRAMS\Codex++\Codex++ 绠＄悊宸ュ叿.lnk"
  Delete "$SMPROGRAMS\Codex++\卸载 Codex++.lnk"
  RMDir "$SMPROGRAMS\Codex++"

  Delete "$INSTDIR\codex-plus-plus.exe"
  Delete "$INSTDIR\codex-plus-plus-manager.exe"

  ; NSIS 的 Delete 遇到被占用文件会静默失败，用户会误以为卸载干净。
  ; 残留检测提示用户先从托盘退出，避免“卸载后重装”继续失败（issue #2362）。
  IfFileExists "$INSTDIR\codex-plus-plus-manager.exe" 0 +2
    MessageBox MB_OK|MB_ICONEXCLAMATION "codex-plus-plus-manager.exe 仍被占用，未能删除。请从系统托盘退出 Codex++（或结束该进程）后重新卸载或安装。"

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
