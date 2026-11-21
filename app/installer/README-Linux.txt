TradePilot for Linux

Installation: unpack the archive, e.g. to ~/Programs/TradePilot, and start ./TradePilot there.
Optionally add it to the application menu:
  - copy TradePilot.desktop to ~/.local/share/applications and enter the full path to TradePilot in the line "Exec=",
  - copy TradePilot.png to ~/.local/share/icons.

Requirements: 64-bit Linux (x86_64) with a graphical desktop (X11 or Wayland) and OpenGL.

Data: ~/.local/share/TradeWorkspace (or $XDG_DATA_HOME/TradeWorkspace): trades, templates, settings, log.
It is not part of the package and is kept when the program is deleted.
Portable mode: put a file portable.txt next to TradePilot; the data then lives in the folder Data beside it.

TWS: Interactive Brokers TWS or Gateway with the API enabled (port 7497 = paper).
