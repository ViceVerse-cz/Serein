; Serein Windows Installer Script (NSIS Modern UI 2)
; Installs per-user to $LOCALAPPDATA\Programs\Serein without elevation
; Preserves write permissions for seamless in-app autoupdates

Unicode True
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define PRODUCT_NAME "Serein"
!define PRODUCT_PUBLISHER "Serein contributors"
!define PRODUCT_WEB_SITE "https://github.com/ViceVerse-cz/Serein"
!define APP_EXE "serein.exe"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!ifndef DIST_DIR
  !if /FileExists "dist"
    !define DIST_DIR "dist"
  !else
    !define DIST_DIR "..\..\dist"
  !endif
!endif

!ifndef OUTPUT_DIR
  !if /FileExists "dist"
    !define OUTPUT_DIR "dist-installer"
  !else
    !define OUTPUT_DIR "..\..\dist-installer"
  !endif
!endif

Name "${PRODUCT_NAME} ${VERSION}"
OutFile "${OUTPUT_DIR}\serein-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Serein"
InstallDirRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "InstallLocation"

!if /FileExists "packaging\windows\Serein.ico"
  !define MUI_ICON "packaging\windows\Serein.ico"
  !define MUI_UNICON "packaging\windows\Serein.ico"
!else if /FileExists "${__FILEDIR__}\Serein.ico"
  !define MUI_ICON "${__FILEDIR__}\Serein.ico"
  !define MUI_UNICON "${__FILEDIR__}\Serein.ico"
!else if /FileExists "Serein.ico"
  !define MUI_ICON "Serein.ico"
  !define MUI_UNICON "Serein.ico"
!else
  !define MUI_ICON "${__FILEDIR__}\Serein.ico"
  !define MUI_UNICON "${__FILEDIR__}\Serein.ico"
!endif

!define MUI_ABORTWARNING

; UI Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${DIST_DIR}\LICENSE-MIT"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${PRODUCT_NAME}"
!insertmacro MUI_PAGE_FINISH

; Uninstaller UI Pages
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}

  ${Do}
    nsExec::Exec 'powershell -NoProfile -NonInteractive -Command "if (Get-Process serein -ErrorAction SilentlyContinue) { exit 1 } else { exit 0 }"'
    Pop $0
    ${If} $0 != 0
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${PRODUCT_NAME} is currently running. Please close Serein before continuing." IDRETRY retry_init IDCANCEL cancel_init
      retry_init:
        ${Continue}
      cancel_init:
        Abort
    ${Else}
      ${Break}
    ${EndIf}
  ${Loop}
FunctionEnd

Section "MainSection" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite on

  ; Copy all package payload files
  File /r "${DIST_DIR}\*.*"

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Create Start Menu shortcut
  CreateDirectory "$SMPROGRAMS"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  ; Register AUMID on Start Menu shortcut for native toast notifications
  nsExec::Exec 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\install-notifications.ps1" -Force'

  ; Write Add/Remove Programs uninstall registry keys
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "NoRepair" 1

  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "EstimatedSize" "$0"
SectionEnd

Function un.onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}

  ${Do}
    nsExec::Exec 'powershell -NoProfile -NonInteractive -Command "if (Get-Process serein -ErrorAction SilentlyContinue) { exit 1 } else { exit 0 }"'
    Pop $0
    ${If} $0 != 0
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${PRODUCT_NAME} is currently running. Please close Serein before uninstalling." IDRETRY retry_uninit IDCANCEL cancel_uninit
      retry_uninit:
        ${Continue}
      cancel_uninit:
        Abort
    ${Else}
      ${Break}
    ${EndIf}
  ${Loop}
FunctionEnd

Section "Uninstall"
  ; Remove Start Menu shortcut
  Delete "$SMPROGRAMS\${PRODUCT_NAME}.lnk"

  ; Remove Desktop shortcut if present
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  ; Remove Run registry key if startup was configured
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCT_NAME}"

  ; Remove Uninstall registry keys
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

  ; Remove installed files
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\install-notifications.ps1"
  Delete "$INSTDIR\setup.ps1"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\docs"
  RMDir /r "$INSTDIR\licenses"
  RMDir /r "$INSTDIR\source"

  ; Remove installation directory if empty or leftover update staging
  RMDir /r "$INSTDIR"
SectionEnd
