; Composer CLI Windows installer. Uses upstream NSIS compiler, not a custom installer.
; Build: makensis /DCOMPOSER_SOURCE=... /DCOMPOSER_OUTPUT=... packaging/composer-cli.nsi
!ifndef COMPOSER_SOURCE
  !error "Missing /DCOMPOSER_SOURCE pointing to the release composer.exe"
!endif
!ifndef COMPOSER_OUTPUT
  !error "Missing /DCOMPOSER_OUTPUT pointing to an installer output path"
!endif

Unicode True
Name "Composer CLI"
OutFile "${COMPOSER_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Composer"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show

!define APP_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ComposerCLI"
!define APP_VERSION "0.1.0"

Section "Composer CLI" SEC_MAIN
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "/oname=composer.exe" "${COMPOSER_SOURCE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Current-user installation only; no elevation or machine-wide changes.
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "DisplayName" "Composer CLI"
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "Publisher" "Composer contributors"
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\composer.exe"
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${APP_UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${APP_UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${APP_UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ; No recursive deletion of arbitrary user files.
  Delete "$INSTDIR\composer.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${APP_UNINSTALL_KEY}"
SectionEnd
