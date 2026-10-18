# Whisp for Windows

A native Windows scratchpad based on [Whisp](https://github.com/tanaybhomia/Whisp):
Rust, the Win32 API and the RichEdit control, so it starts in milliseconds and
needs nothing but Windows itself. Notes are plain `.md` files in
`%LOCALAPPDATA%\whisp\notes`, the same folder Whisp uses.

## Build

Windows builds are cross compiled on Linux (here inside the `whisp` distrobox,
Fedora 44) with these packages: `rust cargo clippy rustfmt
rust-std-static-x86_64-pc-windows-gnu mingw64-gcc mingw64-winpthreads-static`.

```sh
cargo win      # alias for: cargo build --release --target x86_64-pc-windows-gnu
cargo test     # platform independent parts, on the build machine
```

The program is `target/x86_64-pc-windows-gnu/release/whisp.exe`.
