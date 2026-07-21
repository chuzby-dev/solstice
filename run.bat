@echo off
REM Solstice one-click launcher: starts the live paper-trading API server
REM and the React dashboard, then opens the dashboard in your browser.
REM No real transactions are ever made -- this only reads on-chain data
REM and simulates trades on paper.

setlocal
set "ROOT=%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "OPENSSL_DIR=C:\Program Files\OpenSSL-Win64"
set "OPENSSL_LIB_DIR=C:\Program Files\OpenSSL-Win64\lib\VC\x64\MD"
set "OPENSSL_INCLUDE_DIR=C:\Program Files\OpenSSL-Win64\include"

if not exist "%ROOT%.env" (
    echo.
    echo [!] No .env file found at %ROOT%.env
    echo     Add a line like: HELIUS_RPC_URL=https://mainnet.helius-rpc.com/?api-key=YOUR_KEY
    echo.
    pause
    exit /b 1
)

echo Starting Solstice API server (paper trading engine)...
start "Solstice API" cmd /k "cd /d "%ROOT%" && cargo run -p solstice-api --bin serve"

echo Starting Solstice dashboard...
start "Solstice Dashboard" cmd /k "cd /d "%ROOT%dashboard" && npm run dev"

echo Waiting for the dashboard to come up...
timeout /t 10 /nobreak >nul

start http://localhost:5173

echo.
echo Solstice is running. Two windows opened: the API server and the
echo dashboard dev server. Close both windows (or Ctrl+C in each) to stop.
echo.
