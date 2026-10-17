//! Embeds the application icon (resource 1, used by Explorer, the taskbar and the tray)
//! and version information into the Windows executable.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/riftlite.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let var = |k: &str| env::var(k).unwrap_or_else(|_| panic!("{k} is set by cargo"));
    let ico = PathBuf::from(var("CARGO_MANIFEST_DIR")).join("assets").join("riftlite.ico");
    let ico = ico.display().to_string().replace('\\', "/");
    let version = var("CARGO_PKG_VERSION");
    let numeric = format!("{},{},{},0", var("CARGO_PKG_VERSION_MAJOR"), var("CARGO_PKG_VERSION_MINOR"), var("CARGO_PKG_VERSION_PATCH"));
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "riftlite"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "riftlite"
      VALUE "OriginalFilename", "riftlite.exe"
      VALUE "ProductName", "riftlite"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let path = PathBuf::from(var("OUT_DIR")).join("riftlite.rc");
    std::fs::write(&path, rc).expect("write riftlite.rc");
    embed_resource::compile(&path, embed_resource::NONE).manifest_optional().expect("compile riftlite.rc");
}
