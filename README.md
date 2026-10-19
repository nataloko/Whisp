# Whisp for Windows

A native Windows scratchpad based on [Whisp](https://github.com/tanaybhomia/Whisp):
Rust, the Win32 API and the RichEdit control, so it starts in milliseconds and
needs nothing but Windows itself. Notes are plain `.md` files in
`%LOCALAPPDATA%\whisp\notes`, the same folder Whisp uses.

## What works

- One window with a RichEdit editor, following the Windows light or dark app
  theme (or a fixed one), title bar included, and per monitor DPI.
- A carousel of notes as in Whisp: pinned notes first (`Ctrl+Shift+P`), then
  the rest oldest first, with an empty note after the newest. Old notes can
  be archived and the carousel size capped; left out notes stay searchable.
  `Ctrl+]` and `Ctrl+[` move between notes, as do touchpad swipes and the
  mouse wheel at a note's edge. It starts on the note shown at the last close.
- Saving is automatic, 500 ms after the last change and at once when leaving
  a note or closing. A trailing `*` in the title means a save failed; it is
  retried. Blank notes are deleted, as in Whisp.
- Live Markdown highlighting, and Whisp's hide symbols mode (`Ctrl+E`),
  globally or per note.
- Search across all notes (`Ctrl+F`) with tags and match highlights; OCR
  paste of images; checklists; `::` commands and text expansions; paper
  backgrounds (`Alt+P`); Whisp's editing keys and note actions.
- `Ctrl+D` or `Shift+Delete` moves the current note into Whisp's `.trash`
  folder; `Ctrl+Shift+T` brings back the most recently deleted one.
- Preferences (`Ctrl+,`) and a list of keyboard shortcuts (`Ctrl+/`).
- A second launch brings the running window to the front.

Settings are kept in `%LOCALAPPDATA%\whisp-win\settings.ini`; pins and the
per note hide symbols state are in Whisp's own `metadata.json` in the notes
folder, so both apps agree. `WHISP_NOTES_DIR` (the default notes folder) and
`WHISP_SETTINGS_DIR` override the folders, for testing.

## Icon

The icon is Whisp's (`data/icons/io.github.tanaybhomia.Whisp.svg` in Whisp,
GPL-3.0). `assets/whisp.ico` holds PNG frames from 16 to 256 px, made with
ImageMagick (with librsvg) and `assets/pack_ico.py`:

```sh
for s in 256 64 48 40 32 24 20 16; do
  magick -background none -density 1536 io.github.tanaybhomia.Whisp.svg \
    -resize ${s}x$s PNG32:$s.png
done
uv run assets/pack_ico.py assets/whisp.ico 256.png 64.png 48.png 40.png 32.png 24.png 20.png 16.png
```

`build.rs` embeds it, with version information from `Cargo.toml`, through
`x86_64-w64-mingw32-windres` (from `mingw64-binutils`).

## Build

Windows builds are cross compiled on Linux (here inside the `whisp` distrobox,
Fedora 44) with these packages: `rust cargo clippy rustfmt
rust-std-static-x86_64-pc-windows-gnu mingw64-gcc mingw64-winpthreads-static`.

```sh
cargo win      # alias for: cargo build --release --target x86_64-pc-windows-gnu
cargo test     # platform independent parts, on the build machine
```

The program is `target/x86_64-pc-windows-gnu/release/whisp.exe`.

### Installer

`installer/whisp.iss` builds `target/installer/Whisp-<version>-setup.exe`
with [Inno Setup](https://jrsoftware.org/isinfo.php) 7 on Windows, from the
built program (its version comes from the exe):

```bat
ISCC installer\whisp.iss
```

The installer needs no admin rights (it offers an all users install too),
adds a Start menu entry and optionally a desktop icon, and closes a running
Whisp first so it saves. Uninstalling keeps the notes and settings. It
refuses to install over the old GTK build of Whisp.

## Licence

GPL-3.0-or-later, like Whisp, which this program ports to Windows (its
Markdown rules, list and expansion behaviour, texts and icon). See `LICENSE`.
