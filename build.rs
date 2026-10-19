//! Embeds the icon and version information in the Windows executable:
//! writes a resource script, compiles it with windres and links the object.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/whisp.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest.join("assets").join("whisp.ico");
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let mut numbers = version
        .split(['.', '-', '+'])
        .map(|n| n.parse::<u16>().unwrap_or(0));
    let mut next = || numbers.next().unwrap_or(0);
    let (major, minor, patch) = (next(), next(), next());

    // Numbers instead of winver.h names: VOS_NT_WINDOWS32 is 0x40004 and
    // VFT_APP is 1. Resource 1 is the icon the window class loads.
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "ProductName", "Whisp"
      VALUE "FileDescription", "Whisp"
      VALUE "ProductVersion", "{version}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "whisp"
      VALUE "OriginalFilename", "whisp.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
    );
    let rc = out.join("whisp.rc");
    std::fs::write(&rc, script).expect("write whisp.rc");

    // x86_64-pc-windows-gnu builds with x86_64-w64-mingw32-windres; WINDRES
    // overrides it.
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        format!("{arch}-w64-mingw32-windres")
    });
    let object = out.join("whisp-resources.o");
    let status = Command::new(&windres)
        .arg("--input")
        .arg(&rc)
        .arg("--output")
        .arg(&object)
        .arg("--output-format=coff")
        .status()
        .unwrap_or_else(|e| panic!("cannot run {windres}: {e}"));
    assert!(status.success(), "{windres} failed");
    println!("cargo:rustc-link-arg-bins={}", object.display());
}
