; Prefer the registered install directory even when the publisher changes.
InstallDirRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\羽连 PlumeLink" "InstallLocation"
Var PreviousPublisher

!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $PreviousPublisher SHCTX "${UNINSTKEY}" "Publisher"
  ReadRegStr $R7 SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $R7 != ""
    ${GetFileName} "$R7" $0
    ${GetFileExt} "$R7" $1
    ${If} $0 != $R7
    ${OrIf} $1 != "exe"
      Abort "Invalid registered application filename"
    ${EndIf}
    ${If} $R7 != "${MAINBINARYNAME}.exe"
    ${AndIf} ${FileExists} "$INSTDIR\$R7"
      !insertmacro CheckIfAppIsRunning "$R7" "${PRODUCTNAME}"
    ${EndIf}
  ${EndIf}
!macroend

!macro RefreshShortcut LINK
  ${If} ${FileExists} "${LINK}"
    !insertmacro IsShortcutTarget "${LINK}" "$INSTDIR\$OldMainBinaryName"
    Pop $0
    ${If} $0 = 1
      !insertmacro SetShortcutTarget "${LINK}" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${EndIf}
    !insertmacro IsShortcutTarget "${LINK}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro SetLnkAppUserModelId "${LINK}"
    ${EndIf}
  ${EndIf}
!macroend

!macro DeleteAppFirewallRule NAME EXE
  nsExec::ExecToStack '"$SYSDIR\netsh.exe" advfirewall firewall delete rule name="${NAME}" dir=in program="${EXE}"'
  Pop $0
  Pop $1
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"
    ClearErrors
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --update-startup-registration "$OldMainBinaryName"' $0
    ${If} ${Errors}
    ${OrIf} $0 != 0
      ; Preserve the migration source for a retry.
      WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "$OldMainBinaryName"
      SetErrorLevel 1
      Abort "Could not update the startup registration"
    ${EndIf}
    ; Prior releases named their firewall rule after the executable without hyphens.
    ${GetBaseName} "$OldMainBinaryName" $2
    nsis_tauri_utils::StrReplace "$2" "-" ""
    Pop $2
    !insertmacro DeleteAppFirewallRule "$2" "$INSTDIR\$OldMainBinaryName"
    !insertmacro RefreshShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
    !insertmacro RefreshShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${If} $AppStartMenuFolder != ""
      !insertmacro RefreshShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${EndIf}
  ${If} $PreviousPublisher != ""
  ${AndIf} $PreviousPublisher != "${MANUFACTURER}"
    ${GetFileName} "$PreviousPublisher" $0
    ${If} $0 == $PreviousPublisher
      ReadRegStr $0 SHCTX "Software\$PreviousPublisher\${PRODUCTNAME}" ""
      ${If} $0 == $INSTDIR
        DeleteRegValue SHCTX "Software\$PreviousPublisher\${PRODUCTNAME}" ""
        DeleteRegKey /ifempty SHCTX "Software\$PreviousPublisher\${PRODUCTNAME}"
        DeleteRegKey /ifempty SHCTX "Software\$PreviousPublisher"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  !insertmacro DeleteAppFirewallRule "${MAINBINARYNAME}" "$INSTDIR\${MAINBINARYNAME}.exe"
  nsExec::ExecToStack '"$SYSDIR\netsh.exe" advfirewall firewall add rule name="${MAINBINARYNAME}" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=private,domain protocol=TCP remoteip=LocalSubnet'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "Windows Firewall rule requires administrator access: $1"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro DeleteAppFirewallRule "${MAINBINARYNAME}" "$INSTDIR\${MAINBINARYNAME}.exe"
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${MAINBINARYNAME}"
    ${If} $0 == '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${MAINBINARYNAME}"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${MAINBINARYNAME}"
    ${EndIf}
  ${EndIf}
!macroend
