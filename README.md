# eDEX-Remake

eDEX-Remake is a native Linux terminal and system monitor for servers and dedicated displays. It ports the dashboard layout, themes, on-screen keyboard, opening sequence, and sound effects of [EdexUi-2026](https://github.com/Ali-A-Alwahed/EdexUi-2026) to Rust, GTK4, and VTE.

## Differences from EdexUi-2026

This comparison uses EdexUi-2026 2026.4.22 at the [`2321bd5` commit](https://github.com/Ali-A-Alwahed/EdexUi-2026/tree/2321bd5), the basis of the port.

| Area | EdexUi-2026 | eDEX-Remake |
| --- | --- | --- |
| Runtime and build | Electron and Chromium render HTML, CSS, and JavaScript. npm and electron-builder build the app. | Rust and GTK4 render the interface. Cargo builds the native executable. |
| Platforms | Build configurations for Linux, macOS, and Windows | Developed for Linux Wayland sessions and the Cage kiosk compositor |
| Terminal | xterm.js and node-pty connect through Electron IPC, with five tabs | VTE manages local PTYs. Five tabs are available by default, with additional tabs and a toolbar that hides until hovered. |
| Terminal appearance | Theme JSON defines fonts and colors. Settings control font size. | Settings control the terminal font, size, presets, and custom palette. Changes apply to open tabs immediately. |
| Files and apps | A bottom file area and top buttons open the file explorer and installed app list. | SYSTEM switches between shortcuts and a file list that follows the active terminal. The two top buttons can be hidden independently. |
| Visual effects and sound | Browser-based charts, a WebGL globe, and Howler sound effects | GTK and Cairo charts and globe, with SDL3 effects that can overlap during rapid typing |

The GTK and Cairo globe reuses the upstream map tiles, rotation, and opening scan. Tile blending, particles, and text rendering differ from the WebGL version. The theme converter handles a subset of `injectCSS`, including panel colors and opacity, text size and spacing, corner radius, and selected layout rules. Chromium-specific selectors, media queries, and external URL rules have limited compatibility. Screen comparisons used a software-rendered 1920×1080 Cage session.

## Keyboard source fidelity

The GTK4 keyboard ports the input behavior and visual hierarchy of EdexUi-2026 at [`2321bd5`](https://github.com/Ali-A-Alwahed/EdexUi-2026/tree/2321bd5). The upstream references are [`keyboard.class.js`](https://github.com/Ali-A-Alwahed/EdexUi-2026/blob/2321bd5/src/classes/keyboard.class.js) and [`keyboard.css`](https://github.com/Ali-A-Alwahed/EdexUi-2026/blob/2321bd5/src/assets/css/keyboard.css). Key layouts come from the original JSON files already bundled in `assets/keyboards/`.

GTK overlays reproduce the original main/Shift/Alt/Alt+Shift/Fn legend positions, split Enter, wider edge keys and spacebar. Hardware keydown holds the active highlight; keyup triggers the upstream 100ms blink (approximated with GTK timeouts because the Chromium CSS keyframe is not directly supported). Authentication mode keeps the keyboard visible and dimmed, without disclosing pressed keys. Visible outlines on ordinary keys are an intentional eDEX-Remake customization requested after the initial port; the upstream default outlines only selected keys.

## Features

- **System monitoring** — Track CPU utilization, frequency and temperature, memory, swap, disk usage, mounts, uptime, and battery. A 40×11 dot display distinguishes used, reclaimable, and available memory. Sort the process table by PID, name, CPU, memory, and other columns.
- **Network monitoring** — View interface status, addresses, transfer rates and totals, and TCP round-trip latency. Upload traffic appears above the chart baseline and download traffic below it. Connect a GeoLite2 City database to plot peer locations on the globe.
- **Terminal tabs** — The tab bar hides three seconds after the pointer leaves the top of the terminal. Hover there to reveal it. Select **+** or press `Ctrl+Shift+T` to open a tab in the current directory. Scroll the list horizontally when the tabs outgrow the available space.
- **Files and apps** — SYSTEM’s **Files** view follows the active terminal’s working directory, including fish sessions. Clicking a folder at a shell prompt clears the current input line and changes directory. Search files and installed apps, toggle dotfiles, preview images, fonts, PDFs, audio and video, and edit UTF-8 text files.
- **Appearance and input** — Includes 21 upstream themes, 18 keyboard layouts, the boot log and glitch sequence, and sound effects for physical and on-screen keys. Terminal color schemes include **Follow theme**, Catppuccin Latte, Frappé, Macchiato, and Mocha, Nord, and **Custom**. Custom swatches let you set text, background, cursor, selection, and all 16 ANSI colors. [Catppuccin](https://github.com/catppuccin/alacritty) · [Nord](https://www.nordtheme.com/docs/colors-and-palettes/)
- **Password prompts** — The on-screen keyboard hides automatically during supported prompts such as `sudo`, `su`, and `systemd-ask-password` in the active terminal. It returns when normal input resumes. Use `Ctrl+Shift+P` to hide it manually when a remote or nested terminal does not report its echo state to the local PTY.

While **Audio effects** is enabled, eDEX-Remake keeps the SDL3 playback device active between key presses to avoid gaps when the output wakes up.

## Build on Arch Linux

```bash
sudo pacman -S --needed git base-devel rust gtk4 vte4 fontconfig sdl3
git clone https://github.com/ACOPS1206/eDEX-Remake.git
cd eDEX-Remake
cargo build --release --locked
```

The executable is written to `target/release/edex-native`. If you move it outside the checkout, place the repository’s `assets/` directory next to it. Run the unit tests with `cargo test --locked`.

PDF previews require Poppler. Audio and video previews use GTK’s GStreamer backend and the plugins for the media format.

```bash
sudo pacman -S --needed poppler gst-plugins-base gst-plugins-good
```

## Settings and shortcuts

Open settings with `Ctrl+Shift+S` and select **Save to Disk**. Changes to **Terminal font**, **Terminal font size**, and **Terminal colors** apply immediately while preserving running shells and scrollback. **Show top file manager button** and **Show top installed apps button** control the two header buttons independently. Both features also remain accessible from SYSTEM and their shortcuts.

Enter a color such as `#121820` in **Terminal background** to override the background of the selected terminal color scheme. Clear the field and save to restore the scheme’s background.

Settings are stored under `$XDG_CONFIG_HOME` or `~/.config`. The app reuses an existing `EdexUi-2026/settings.json` or `eDEX-UI/settings.json` when present. It also reads supported app actions and shell command shortcuts from the upstream `shortcuts.json` format.

| Keys | Action |
| --- | --- |
| `Ctrl+1`–`Ctrl+5` | Select a default terminal tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous terminal tab |
| `Ctrl+Shift+T` | Add a terminal tab |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste in the terminal |
| `Ctrl+Shift+S` / `Ctrl+Shift+K` | Settings / shortcuts |
| `Ctrl+Shift+A` / `Ctrl+Shift+F` | Installed apps / file search |
| `Ctrl+Shift+L` / `Ctrl+Shift+H` | File list / dotfiles |
| `Ctrl+Shift+P` | Manually toggle on-screen keyboard privacy |
| `F9` / `F10` / `F11` | Bottom panels / side panels / fullscreen |
| `Ctrl+Shift+Q` | Quit |

### GeoIP map

In settings, point to an existing GeoLite2 City file or select **Download GeoLite2 City**. The downloader uses `curl`, `tar`, and `sha384sum` to fetch, unpack, and verify the file before storing it in the configuration directory’s `geoIPcache` folder. Peer addresses come from the Linux TCP tables and are looked up in the local database. If a database is available and the interface has no public address, the app uses an external IP lookup service to identify its own endpoint. **Map public IPv4 addresses in terminal output** enables local lookups for public IPv4 addresses printed in the terminal. MaxMind provides the GeoLite2 data.

## Upstream and license

EdexUi-2026 is based on [GitSquared/eDEX-UI](https://github.com/GitSquared/edex-ui). See [LICENSE](LICENSE) for the license text.
