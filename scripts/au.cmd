@echo off
setlocal
set "AU_EXE=%~dp0..\crates\android-use\target\release\au.exe"
if not exist "%AU_EXE%" (
  echo err E_BUILD au.exe is not built; run scripts\build-au.cmd
  exit /b 2
)
"%AU_EXE%" %*
exit /b %ERRORLEVEL%
