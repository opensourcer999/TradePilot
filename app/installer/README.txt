TradePilot - Quick Guide
========================

TradePilot manages options trades through Interactive Brokers' Trader Workstation (TWS) or IB Gateway.
Orders are only sent to paper trading accounts (account number starts with "DU").

1. Requirements
   - Windows 10 or 11, 64-bit. Nothing else needs to be installed.
   - TWS or IB Gateway is running and logged in (preferably a paper trading account).
   - The API is enabled in TWS:
       File > Global Configuration > API > Settings
       switch on "Enable ActiveX and Socket Clients".
       The socket port must match TradePilot (default: paper 7497, live 7496).
       If TWS runs on another computer: add that computer's address under "Trusted IPs"
       and enter host and port in TradePilot under Settings.
   - Market data subscriptions for the prices TradePilot needs (SPX, VIX and options).

2. Starting
   - Start TradePilot and click "Connect to TWS" at the top.
   - On the first start, settings, templates and schedules are empty.

3. Where is the data?
   Portable ZIP (the file portable.txt lies next to TradePilot.exe): in the folder Data next to the EXE. The folder is
   empty on the first start; the data of another installation is not visible. To move it, copy the whole folder (with
   the app closed).
   Setup, or a single EXE without portable.txt:
   %LOCALAPPDATA%\TradeWorkspace      (e.g. C:\Users\<Name>\AppData\Local\TradeWorkspace)
     paper\          database (trades, executions, commissions), templates, schedules, settings
     logs\           log
     backups\        backups
   This data is not part of the installation package and is kept when uninstalling.
   To move to another computer: copy the folder TradeWorkspace with the app closed.
   Note: the settings also contain the e-mail credentials.

4. Important
   - Never run TradePilot on two computers at the same time with the same account and the same client ID.
   - Every other API application on the same TWS needs a different client ID (TradePilot's default: 1).
