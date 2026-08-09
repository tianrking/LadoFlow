!macro LadoFlowRequireSetupSuccess STEP
  Pop $0
  ${If} $0 == "3010"
    DetailPrint "LadoFlow ${STEP} succeeded and requested a restart."
    SetRebootFlag true
  ${ElseIf} $0 != "0"
    DetailPrint "LadoFlow ${STEP} failed with exit code $0."
    MessageBox MB_ICONSTOP|MB_OK "LadoFlow could not complete ${STEP} (exit code $0). The installer will stop without changing Windows boot or certificate settings."
    Abort
  ${Else}
    DetailPrint "LadoFlow ${STEP} succeeded."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Tauri invokes PREINSTALL before its own process check. Close the desktop
  ; process before stopping the service so cancelling the prompt cannot leave
  ; an otherwise intact installation partially disabled.
  !insertmacro CheckIfAppIsRunning "ladoflow-desktop.exe" "LadoFlow"
  ${If} ${FileExists} "$INSTDIR\windows\LadoFlowWindowsSetup.exe"
    DetailPrint "Preparing the existing LadoFlow virtual-display service for upgrade..."
    nsExec::ExecToLog '"$INSTDIR\windows\LadoFlowWindowsSetup.exe" prepare-install'
    !insertmacro LadoFlowRequireSetupSuccess "upgrade preparation"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${FileExists} "$INSTDIR\windows\LadoFlowWindowsSetup.exe"
    MessageBox MB_ICONSTOP|MB_OK "The LadoFlow Windows setup helper is missing. The installer cannot safely register the virtual display."
    Abort
  ${EndIf}
  DetailPrint "Installing the LadoFlow virtual-display driver and service..."
  nsExec::ExecToLog '"$INSTDIR\windows\LadoFlowWindowsSetup.exe" install'
  !insertmacro LadoFlowRequireSetupSuccess "virtual-display installation"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CheckIfAppIsRunning "ladoflow-desktop.exe" "LadoFlow"
  ${IfNot} ${FileExists} "$INSTDIR\windows\LadoFlowWindowsSetup.exe"
    MessageBox MB_ICONSTOP|MB_OK "The LadoFlow Windows setup helper is missing. Reinstall LadoFlow before uninstalling so its driver can be removed safely."
    Abort
  ${EndIf}
  ${If} $UpdateMode == 1
    DetailPrint "Stopping the LadoFlow virtual-display service for an application update..."
    nsExec::ExecToLog '"$INSTDIR\windows\LadoFlowWindowsSetup.exe" prepare-install'
    !insertmacro LadoFlowRequireSetupSuccess "update preparation"
  ${Else}
    DetailPrint "Removing the LadoFlow virtual-display service and recorded driver packages..."
    nsExec::ExecToLog '"$INSTDIR\windows\LadoFlowWindowsSetup.exe" uninstall'
    !insertmacro LadoFlowRequireSetupSuccess "virtual-display removal"
  ${EndIf}
!macroend
