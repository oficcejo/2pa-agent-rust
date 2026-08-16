@echo off
chcp 65001 >nul
title OKX 2PA Agent (Rust Edition)
echo ====================================================
echo   OKX 2PA Agent (Rust High-Performance Edition)
echo   Starting server at http://127.0.0.1:8088 ...
echo ====================================================

REM Automatically add Cargo bin directory to PATH if not already present
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

if exist "target\release\okx-2pa-agent.exe" (
    "target\release\okx-2pa-agent.exe" --host 127.0.0.1 --port 8088
) else (
    cargo run --release -- --host 127.0.0.1 --port 8088
)
pause
