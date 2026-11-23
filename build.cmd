@echo off
setlocal
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\build.ps1"
if errorlevel 1 (
  echo Build failed. See the error above.
  pause
  exit /b 1
)
echo.
echo Done: AgentVoice.exe and versioned portable package updated.
pause
