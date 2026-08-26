#!/usr/bin/env pwsh
Set-Location "$PSScriptRoot\src-tauri"
cargo tauri dev @args
