@echo off
start "" /b powershell.exe -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 5; Set-Content -LiteralPath ([IO.Path]::Combine($env:DOTNET_BUNDLE_EXTRACT_BASE_DIR, 'child.txt')) -Value child-survived"
powershell.exe -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"
