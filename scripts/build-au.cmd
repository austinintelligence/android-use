@echo off
setlocal
pushd "%~dp0..\crates\android-use"
cargo build --release
set "AU_EXIT=%ERRORLEVEL%"
popd
exit /b %AU_EXIT%
