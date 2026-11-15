# Windows package (WSL2 + WSLg)

**This is not a native Windows port of eDEX-Remake.** The included
`eDEX-Remake.exe` is a small Windows executable that launches the original
GTK4/VTE/Linux application inside WSL2.

## Requirements

- Windows 11, or Windows 10 build 19044+ with an up-to-date WSLg installation
- 64-bit x86 PC with virtualization enabled
- WSL2, WSLg, and a distro named `Debian` running **Debian 13**
- Internet access during first-time installation

## First-time setup

1. In PowerShell, install Debian WSL if you do not have it:

   ```powershell
   wsl --install -d Debian
   wsl --update
   ```

   Follow the prompts; a reboot may be required. Start Debian once and create
   its Linux user account. Check the version with `wsl -d Debian -- cat /etc/os-release`.
   The `VERSION_ID` must equal `13`. Not all preinstalled Debian distros use
   this version, and the launcher will refuse an incompatible distro.

2. Extract the **entire** Windows ZIP to a directory you control.
   Do not run the executable from inside the ZIP.

3. Open PowerShell in that directory and run:

   ```powershell
   .\eDEX-Remake.exe --setup
   ```

   This installs GTK4, VTE GTK4, SDL3 and fontconfig inside Debian using
   apt and copies the included Linux application to
   `~/.local/share/eDEX-Remake` in your WSL home. Your Debian sudo password
   may be requested.

4. Double-click `eDEX-Remake.exe` (without arguments). The actual
   eDEX-Remake program runs under WSLg in a Windows desktop window.

## Limitations

- The Windows executable is a **launcher**, not a build of the GTK/VTE app
  for Win32. The terminal uses Linux shells, **not** cmd.exe or PowerShell.
- Network/CPU/RAM/process panels display information from **WSL Linux**,
  not native Windows.
- Access mounted Windows drives through `/mnt/c/`, `/mnt/d/`, etc.
- Do not expect full performance or audio/graphics feature parity with a
  native Windows application; WSLg and drivers can affect rendering.
- WSL itself is not installed automatically by this package.
- To upgrade, extract a newer Windows ZIP and rerun
  `.\eDEX-Remake.exe --setup`.

A genuine native Windows build requires a Windows terminal backend such as
ConPTY plus a Windows-compatible terminal renderer to replace GTK VTE,
and porting Linux-specific `/proc` and `/sys` system telemetry.
