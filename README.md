# Whisp for Windows

A native Windows scratchpad based on [Whisp](https://github.com/tanaybhomia/Whisp):
Rust, the Win32 API and the RichEdit control, so it starts in milliseconds and
needs nothing but Windows itself. Notes are plain `.md` files in
`%LOCALAPPDATA%\whisp\notes`, the same folder Whisp uses.

## What works

- One window with a RichEdit editor, following the Windows light or dark app
  theme (title bar included) and per monitor DPI.
- Notes are ordered by modified time, oldest first, with an empty note after
  the newest. It starts on the newest note. `Ctrl+]` goes to the next (newer)
  note, `Ctrl+[` to the previous one, `Ctrl+N` to the empty note at the end.
  The title shows the position, for example `Whisp  3/12`.
- Saving is automatic, 500 ms after the last change and at once when leaving
  a note or closing. A trailing `*` in the title means a save failed; it is
  retried. Blank notes are deleted, as in Whisp.
- Live Markdown highlighting: headings, bold, italic, strikethrough, inline
  code and code blocks, lists, checkboxes, links, quotes and `//` comments.
- Plain text only: paste drops formatting and RichEdit's formatting
  shortcuts are blocked.
- A second launch brings the running window to the front.

`WHISP_NOTES_DIR` overrides the notes folder, for testing.

## Build

Windows builds are cross compiled on Linux (here inside the `whisp` distrobox,
Fedora 44) with these packages: `rust cargo clippy rustfmt
rust-std-static-x86_64-pc-windows-gnu mingw64-gcc mingw64-winpthreads-static`.

```sh
cargo win      # alias for: cargo build --release --target x86_64-pc-windows-gnu
cargo test     # platform independent parts, on the build machine
```

The program is `target/x86_64-pc-windows-gnu/release/whisp.exe`.
