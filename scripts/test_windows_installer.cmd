@echo off
python scripts\build-difftastic.py --output target\release\helpers
if errorlevel 1 exit /b 1
python scripts\check-difftastic.py --helper target\release\helpers\difft.exe --notices target\release\helpers\difftastic-NOTICES.txt
if errorlevel 1 exit /b 1
cargo wix crates\gitcomet\Cargo.toml -p gitcomet --profile release --nocapture --output dist\gitcomet-local-test.msi
