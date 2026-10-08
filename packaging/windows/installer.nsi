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
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"

!define PRODUCT_NAME "Serein"
!define PRODUCT_PUBLISHER "Serein contributors"
!define PRODUCT_WEB_SITE "https://github.com/ViceVerse-cz/Serein"
!define APP_EXE "serein.exe"
!define APP_ID "cz.viceverse.serein"

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

; The installer uses the x86 NSIS stub, including on x64 and ARM64 Windows.
!if ${NSIS_PTR_SIZE} != 4
  !error "The process-entry layout requires the x86 NSIS installer stub."
!endif

; Returns 0 when absent, 1 when running, 2 when the snapshot cannot be checked.
!macro SereinProcessCheck PREFIX
Function ${PREFIX}CheckSereinRunning
  System::Store "s"
  StrCpy $R0 2
  System::Call 'kernel32::CreateToolhelp32Snapshot(i 2, i 0) p .r0'
  ${If} $0 != -1
    System::Alloc 556
    Pop $1
    ${If} $1 != 0
      ; PROCESSENTRY32W: dwSize followed by 32 bytes of fields and WCHAR[260].
      System::Call '*$1(i 556)'
      System::Call 'kernel32::Process32FirstW(p r0, p r1) i .r2 ?e'
      Pop $3
      ${DoWhile} $2 != 0
        System::Call '*$1(&v36, &w260 .r4)'
        ${If} $4 == "${APP_EXE}"
          StrCpy $R0 1
          ${Break}
        ${EndIf}
        System::Call 'kernel32::Process32NextW(p r0, p r1) i .r2 ?e'
        Pop $3
      ${Loop}
      ${If} $2 == 0
      ${AndIf} $3 == 18 ; ERROR_NO_MORE_FILES, not an enumeration failure.
        StrCpy $R0 0
      ${EndIf}
      System::Free $1
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r0)'
  ${EndIf}
  Push $R0
  System::Store "l"
FunctionEnd
!macroend
!insertmacro SereinProcessCheck ""
!insertmacro SereinProcessCheck "un."

Function RegisterNotificationShortcut
  System::Store "s"
  StrCpy $R0 2
  StrCpy $1 0
  ; CreateShortcut initializes COM. Update the saved shortcut's native property store.
  System::Call 'shell32::SHGetPropertyStoreFromParsingName(w "$SMPROGRAMS\${PRODUCT_NAME}.lnk", p 0, i 2, g "${IID_IPropertyStore}", *p .r1) i .r0'
  ${If} $0 >= 0
    StrCpy $4 0
    System::Call 'shlwapi::SHStrDupW(w "${APP_ID}", *p .r4) i .r0'
    ${If} $0 >= 0
      System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID}) p .r2'
      System::Alloc ${SYSSIZEOF_PROPVARIANT}
      Pop $3
      ${If} $2 != 0
      ${AndIf} $3 != 0
        System::Call '*$3(&i2 ${VT_LPWSTR}, &i6 0, p r4)'
        ${IPropertyStore::SetValue} $1 '($2, $3) .r0'
        ${If} $0 >= 0
          ${IPropertyStore::Commit} $1 '() .r0'
          ${If} $0 >= 0
            StrCpy $R0 0
          ${EndIf}
        ${EndIf}
      ${EndIf}
      System::Free $3
      System::Free $2
    ${EndIf}
    System::Call 'ole32::CoTaskMemFree(p r4)'
    ${IUnknown::Release} $1 ''
  ${EndIf}
  Push $R0
  System::Store "l"
FunctionEnd

Function .onInit
  SetShellVarContext current
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}

  ${Do}
    Call CheckSereinRunning
    Pop $0
    ${If} $0 == 2
      MessageBox MB_OK|MB_ICONSTOP "Cannot check whether ${PRODUCT_NAME} is running. Setup cannot continue safely." /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $0 != 0
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${PRODUCT_NAME} is currently running. Please close Serein before continuing." /SD IDCANCEL IDRETRY retry_init IDCANCEL cancel_init
      retry_init:
        ${Continue}
      cancel_init:
        SetErrorLevel 1
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
  File /r /x install-notifications.ps1 /x setup.ps1 "${DIST_DIR}\*.*"
  ; Remove legacy installer utilities when upgrading an existing installation.
  Delete "$INSTDIR\install-notifications.ps1"
  Delete "$INSTDIR\setup.ps1"

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

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

  ; Keep uninstall registration available even if shortcut creation fails.
  ClearErrors
  CreateDirectory "$SMPROGRAMS"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  StrCpy $0 2
  ${IfNot} ${Errors}
    Call RegisterNotificationShortcut
    Pop $0
  ${EndIf}
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "Cannot register the ${PRODUCT_NAME} notification shortcut. You can remove the incomplete installation from Windows Installed Apps." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
SectionEnd

Function un.onInit
  SetShellVarContext current
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}

  ${Do}
    Call un.CheckSereinRunning
    Pop $0
    ${If} $0 == 2
      MessageBox MB_OK|MB_ICONSTOP "Cannot check whether ${PRODUCT_NAME} is running. Uninstall cannot continue safely." /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $0 != 0
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${PRODUCT_NAME} is currently running. Please close Serein before uninstalling." /SD IDCANCEL IDRETRY retry_uninit IDCANCEL cancel_uninit
      retry_uninit:
        ${Continue}
      cancel_uninit:
        SetErrorLevel 1
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
