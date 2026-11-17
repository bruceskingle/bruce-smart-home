; Included in the Windows installer, which dx bundle's NSIS script does between its install and uninstall sections, see
; Dioxus.toml's installer_hooks. dx 0.7.10 includes the file but never calls the NSIS_HOOK_ macros of Tauri's
; installer, and leaves out external_bin on Windows, as it looks for each without .exe, so this adds a hidden section,
; run after dx's own, which puts the virtual capture device, its launcher and the firmware for each board it simulates,
; see capture/README.md, beside the app's own program, where its Devices page starts it from. They are taken, when the
; installer is built, from the folder SIM_BIN names, an environment variable the release workflow sets to
; capture/target-sim/bin, which capture/build-sim.sh fills; without it, e.g. on a developer's computer, the installer is
; built without them, with a warning. The uninstaller removes them with the rest, as it removes the whole folder.

Section "-Virtual capture device"
  SetOutPath "$INSTDIR"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-sim.exe"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-rlcd42.exe"
  File /nonfatal "$%SIM_BIN%\bruce-smart-home-capture-guitron.exe"
SectionEnd
