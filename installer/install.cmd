@echo off
rem Synector bootstrap installer. Runs installer\install.ps1 next to this file.
rem Usage: installer\install.cmd [menu|setup|portable|uninstall|status|help] [options]
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
set "RC=%ERRORLEVEL%"
rem Keep the window open when the script is started by double-click (no arguments).
if "%~1"=="" pause
exit /b %RC%
