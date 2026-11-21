# TradePilot

![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)
![Language: Rust](https://img.shields.io/badge/language-Rust-orange.svg)
![Platform: Windows](https://img.shields.io/badge/platform-Windows-lightgrey.svg)

TradePilot is a free and open-source desktop application for options trading with
Interactive Brokers' Trader Workstation (TWS), written in Rust (egui). The complete source
code is published under the Apache License 2.0: anyone may use, study, change and pass it
on, also commercially (see [License](#license)). Its features are implemented step by step.

TradePilot is an independent project and is not affiliated with or endorsed by Interactive
Brokers.

**Orders can only be sent for a detected paper trading account.** Live mode does not
allow order submission. TradePilot is not investment advice; use it at your own risk.

## Features

- **Connection** to TWS only: account data, positions, market data and option chains.
- **Trade templates:** create, edit, duplicate, import and export; stored safely (see
  "Data").
- **Instant Order:** a template trade (preview from the live option chain, confirmation,
  one combo order for multi-leg trades, fill attempts) or a single option, stock or
  future entered by hand.
- **Stops and profit targets:** placed once a template trade is filled (stop types,
  REL stops, guardrails, several profit target levels, stop adjustments), closing trades
  by hand, reconciliation with TWS after a connect.
- **Dashboard and reports:** live P/L, premium sold and captured, PCR, win rate, delta,
  today's trades, reports with equity curve, daily view and CSV export; trade history
  rebuilt from TWS executions.
- **Automatic entries:** a template trade can be entered at a set time on set days without
  anyone present, optionally only when a market condition holds. It goes through the same
  safety checks and the same order path as a trade entered by hand. Several schedules can
  be kept side by side and exchanged as files; the Dashboard shows what is due today.
- **Triggers, exits and re-entries:** webhook and EMA-crossover triggers, timed exits, early exits on conditions, progressive limit closing, re-entries, custom trades and conditions on market values.
- **News:** economic news (US first, then German and UK) on its own page and as a bulletin on the Dashboard, translated to German when the screens are German.
- **Light and Advanced mode:** Light shows only Dashboard, Instant Order, Trade Templates, Schedules, News and Settings; switchable in the sidebar.
- **Metal design:** one look for the whole app, modelled on brushed and polished metal: lit steel plates and frames, raised metal buttons, inset input fields, analogue dial instruments with glowing needles, LED status lights.


## Safety

- Every order takes one path to TWS: the safety gate (paper account only, kill switch
  off), the risk check (maximum order quantity), then the hand-over to the connection. An
  order that is refused or cannot be handed over is logged as `WARNING: order N NOT sent`
  and never recorded on a trade.
- Immediate transmission is off by default; a confirmation window shows account, legs,
  limit and quantity before anything is sent. An order held in TWS for review is never
  changed or cancelled by TradePilot.
- The **kill switch** (Instant Order page) blocks every further order and cancels every
  order of TradePilot that may still be working, including stops; the log names the open
  trades left without a stop.
- A template trade is saved and counted only once TWS reports it completely filled.

## Requirements

- Windows, the [Rust toolchain](https://rustup.rs/) (version fixed in
  `app/rust-toolchain.toml`) and the Microsoft C++ Build Tools (MSVC).
- The Rust component `clippy` (`rustup component add clippy`).
- For the broker connection: a running TWS with the socket API enabled and a paper
  trading account. Market data needs the matching permissions at Interactive Brokers.

## Build and run

From the project directory:

```powershell
.\build.cmd              # release build
.\build.cmd --debug      # release and debug build
.\build.cmd -Run         # release build, then start it
.\build.cmd -SkipClippy  # without the separate Clippy pass (faster)
```

`build.cmd` (also works where PowerShell scripts are blocked) calls `app\build.ps1`. The
script runs Clippy, builds in `app\target\release` (debug: `app\target\debug`) and names
the executable `TradePilot-V<version>.exe` (`..._debug.exe`). The application is
recompiled on every call, also for an unchanged version; dependencies stay cached. Older
versioned copies are removed only after the new one exists. Only the debug version:
`app\build.ps1 -Debug`. The script ends running TradePilot processes first.

A build in a folder that OneDrive or a virus scanner watches can be many times slower; set
`CARGO_TARGET_DIR` to a folder elsewhere and the script builds and looks there.

## Tests and checks

From `app/`:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Plus `cargo audit` and `cargo deny check` (see `app/deny.toml`). Tests that need a
real TWS are ignored.

## TWS connection

1. Start TWS, log in to the paper trading account and enable the socket API.
2. In TradePilot choose Paper Trading (switch at the top right) and set host and port
   (defaults `127.0.0.1` and `7497`, they must match TWS).
3. Click "Connect to TWS" (top left). The bar shows the connection, the account type and
   the SPX price.

Outside market hours TWS sends no live index ticks; TradePilot then shows the last daily
close (display only, never used as a trading price). If no prices arrive although the
connection is up, the same user is often logged in elsewhere (IBKR Mobile, Client Portal,
a second TWS; error 10197): log out there and reconnect.

## Passing the app on

```powershell
.\package.cmd            # release build, then a ZIP and (with Inno Setup) a setup program in app\dist
.\package.cmd -NoBuild   # package the release executable that exists already
```

The release build links the C runtime into the executable, so it runs on a Windows 10/11
computer without the Visual C++ redistributable; `package.ps1` refuses an executable that
still needs it. `TradePilot-<version>-portable.zip` holds `TradePilot.exe`, a `portable.txt` and a short guide
(`app\installer\README.txt`). The file `portable.txt` next to the program switches the **portable mode** on: the data
(settings, templates, trades, log) then lives in the folder `Data` next to the program instead of in
`%LOCALAPPDATA%\TradeWorkspace`, so the ZIP can be unpacked anywhere (a USB stick, say) and never touches or shows the data
of another installation. Without the file (the setup program, or a lone `TradePilot.exe`) the data is in the user folder as
before. `TradePilot-<version>-Setup.exe` needs Inno Setup 6
(`winget install JRSoftware.InnoSetup`); it installs without administrator rights, adds a Start
menu entry (a desktop icon on request) and an uninstaller. Neither package contains user data (the portable one starts with an empty `Data` folder):
settings, trades and the log stay in `%LOCALAPPDATA%\TradeWorkspace` and survive an uninstall.
The receiving computer needs TWS or IB Gateway with the API enabled.

## Data

On Windows, settings, logs and data are in `%LOCALAPPDATA%\TradeWorkspace`; Paper Trading
has its own `paper` subfolder (settings, templates, schedules, trades, executions). Trades and
executions are in the SQLite database `tradepilot.db`. Templates and schedules are
written as a whole (temporary file, then renamed); a file that cannot be read is set aside
as `<file>.unreadable-<date-time>`, never overwritten, and saving is blocked if even that
fails. The folder can be changed under Settings ("Database and log file storage
location"); it applies from the next start. This data and any credentials do not belong in
a public repository.

## Project structure

- `app/trade_pilot` - application, user interface, and everything that touches files, the network or TWS (storage, news, mail, webhooks).
- `app/strategy` - the pure trade logic without any I/O or screen: entry (strike selection, combo orders), exit (stops, profit targets, timed and early exits), conditions, templates and their editor form, trade rebuilding from executions, reconciliation, fees, pricing, reports, market calendar. Public items are documented; what comes in as an argument (including the time) is what makes it testable.
- `app/core` - shared types and calculations.
- `app/risk` - risk functions.
- `app/broker` - broker and TWS integration.
- `app/winui` - Windows-specific functions.

## Contributing

Bug reports, ideas and pull requests are welcome. Before a pull request, run the checks
under "Tests and checks". Contributions are accepted under
the same license as the project (Apache-2.0, section 5).

## License

TradePilot is open-source software, licensed under the
[Apache License, Version 2.0](LICENSE-APACHE). It comes without any warranty; trading
options involves substantial risk of loss.
