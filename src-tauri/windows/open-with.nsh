; Adds Trimmy to Explorer's "Open with" menu for video files, without making it the default app.
;
; Tauri's own fileAssociations would write each extension's default handler (Classes\.mp4 "")
; and take over double-click from the user's player, so Trimmy registers only:
;   - a ProgID, Trimmy.Video, that opens a file in Trimmy
;   - that ProgID under each extension's OpenWithProgids (the "Open with" list)
;   - Applications\trimmy.exe with its SupportedTypes and a friendly name
; Everything is per-user (HKCU), like the install itself, and removed on uninstall.

!define TRIMMY_PROGID "Trimmy.Video"
!define TRIMMY_APP_KEY "Software\Classes\Applications\${MAINBINARYNAME}.exe"

!macro TRIMMY_ADD_TYPE EXT
  WriteRegStr HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${TRIMMY_PROGID}" ""
  WriteRegStr HKCU "${TRIMMY_APP_KEY}\SupportedTypes" ".${EXT}" ""
!macroend

!macro TRIMMY_REMOVE_TYPE EXT
  DeleteRegValue HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${TRIMMY_PROGID}"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}"
!macroend

; Keep this list in sync with tauri.macos.conf.json and VIDEO_EXTENSIONS in src/App.svelte.
!macro TRIMMY_EACH_TYPE MACRO
  !insertmacro ${MACRO} "mp4"
  !insertmacro ${MACRO} "m4v"
  !insertmacro ${MACRO} "mov"
  !insertmacro ${MACRO} "mkv"
  !insertmacro ${MACRO} "webm"
  !insertmacro ${MACRO} "avi"
  !insertmacro ${MACRO} "ts"
  !insertmacro ${MACRO} "m2ts"
  !insertmacro ${MACRO} "mts"
  !insertmacro ${MACRO} "wmv"
  !insertmacro ${MACRO} "flv"
  !insertmacro ${MACRO} "3gp"
  !insertmacro ${MACRO} "mpg"
  !insertmacro ${MACRO} "mpeg"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\${TRIMMY_PROGID}" "" "Video"
  WriteRegStr HKCU "Software\Classes\${TRIMMY_PROGID}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr HKCU "Software\Classes\${TRIMMY_PROGID}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  WriteRegStr HKCU "${TRIMMY_APP_KEY}" "FriendlyAppName" "${PRODUCTNAME}"
  WriteRegStr HKCU "${TRIMMY_APP_KEY}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  !insertmacro TRIMMY_EACH_TYPE TRIMMY_ADD_TYPE
  ; SHCNE_ASSOCCHANGED, so Explorer picks the change up without a restart.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; An update runs the old uninstaller first; the new version's install writes the same keys again.
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\Classes\${TRIMMY_PROGID}"
    DeleteRegKey HKCU "${TRIMMY_APP_KEY}"
    !insertmacro TRIMMY_EACH_TYPE TRIMMY_REMOVE_TYPE
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ${EndIf}
!macroend
