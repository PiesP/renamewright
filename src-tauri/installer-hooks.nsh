; Tauri 2.11.4 only evaluates its built-in downgrade guard reliably through
; the reinstall UI. Enforce Renamewright's stricter policy in the install
; section as well so silent and interactive invocations have one contract.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R8 SHCTX "${UNINSTKEY}" "DisplayVersion"
  ${If} $R8 != ""
    nsis_tauri_utils::SemverCompare "${VERSION}" $R8
    Pop $R9
    ${If} $R9 = -1
      SetErrorLevel 2
      Quit
    ${EndIf}
  ${EndIf}
!macroend
