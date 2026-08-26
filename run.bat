@echo off
cd /d "%~dp0"
cargo run --release -- "%~dp0video\Bad Apple.mp4"
pause
