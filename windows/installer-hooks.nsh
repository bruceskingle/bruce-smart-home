; Hooks of the Windows installer, which dx bundle's NSIS installer runs, see Dioxus.toml's installer_hooks: the virtual
; capture device, its launcher and the firmware for each board it simulates, see capture/README.md, put beside the
; app's own program, where its Devices page starts it from, as the installer leaves out Dioxus.toml's external_bin.
; They are taken, when the installer is built, from the folder SIM_BIN names, an environment variable the release
; workflow sets to capture/target-sim/bin, which capture/build-sim.sh fills; without it, e.g. on a developer's
; computer, the installer is built without them, with a warning.

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-sim.exe"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-rlcd42.exe"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-guitron.exe"
!macroend

; Removed with the app, as the installer only removes the files it knows of.
!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\bruce-smart-home-capture-sim.exe"
  Delete "$INSTDIR\bruce-smart-home-capture-rlcd42.exe"
  Delete "$INSTDIR\bruce-smart-home-capture-guitron.exe"
!macroend
