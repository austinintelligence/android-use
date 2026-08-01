@echo off
setlocal
set "AU=%~dp0..\crates\android-use\target\release\au.exe"
if not exist "%AU%" (
  echo err E_BUILD au.exe is not built; run scripts\build-au.cmd
  exit /b 2
)
if "%~1"=="" goto :help

rem Only argument-free compatibility calls are forwarded. cmd.exe cannot
rem preserve arbitrary structured arguments through %%*, so all other calls
rem receive an explicit migration error and never reach a legacy PowerShell
rem implementation.
if /I "%~1"=="d" if "%~2"=="" "%AU%" d & exit /b %ERRORLEVEL%
if /I "%~1"=="devices" if "%~2"=="" "%AU%" d & exit /b %ERRORLEVEL%
if /I "%~1"=="doc" if "%~2"=="" "%AU%" doctor & exit /b %ERRORLEVEL%
if /I "%~1"=="doctor" if "%~2"=="" "%AU%" doctor & exit /b %ERRORLEVEL%
if /I "%~1"=="st" if "%~2"=="" "%AU%" st & exit /b %ERRORLEVEL%
if /I "%~1"=="status" if "%~2"=="" "%AU%" st & exit /b %ERRORLEVEL%
if /I "%~1"=="serve" goto :removed
if /I "%~1"=="scene" goto :removed
if /I "%~1"=="sc" goto :removed
if /I "%~1"=="clear" goto :removed
if /I "%~1"=="url" goto :removed
echo err E_MIGRATION use au.exe %~1; aad argument forwarding is disabled for this compatibility cycle
exit /b 2

:removed
echo err E_MIGRATION canvas commands were removed; use au for Android control
exit /b 2

:help
echo err E_MIGRATION aad is a temporary shim; use au.exe help
exit /b 2
