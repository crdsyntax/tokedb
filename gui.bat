@echo off
cd /d "%~dp0src-tauri"
cargo tauri dev %*
