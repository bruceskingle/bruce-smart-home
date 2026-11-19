@echo off
rem Brings the repo up to date on the latest version's branch and downloads the installer for this PC from the latest
rem try or release, with bruce-method-release's sync and download, see docs/Release.md. Run it from PowerShell, or a
rem Command Prompt, in any directory, e.g. .\scripts\sync-and-download.cmd
rem
rem It needs bruce-method-release, installed with: cargo install bruce-method-release
rem and gh, installed and signed in with: gh auth login
setlocal

rem The repo's checkout, the folder above this script's.
set "REPO=%~dp0.."

bruce-method-release --repo "%REPO%" sync || exit /b 1
bruce-method-release --repo "%REPO%" download || exit /b 1
