//! macOS: embeds a minimal Info.plist in the executable's `__TEXT,__info_plist`
//! section so an unbundled run (`cargo run`) shows "Rift" in the menu bar
//! instead of the binary name. Inside `Rift.app` the bundle's own Info.plist
//! (packaging/make-app.sh) takes precedence.
//!
//! Windows (GNU toolchain): embeds Rift's icon and version details as a
//! resource, so Explorer, the Start menu and the taskbar show Rift's icon and
//! the file's properties name Rift. It needs `windres` (mingw-w64:
//! `x86_64-w64-mingw32-windres`, or `$WINDRES`); without one the build goes
//! on with a warning and a plain executable.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => macos(),
        Ok("windows") => windows(),
        _ => {}
    }
}

fn macos() {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Rift</string>
    <key>CFBundleDisplayName</key><string>Rift</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
</dict>
</plist>
"#
    );
    let path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("Info.plist");
    fs::write(&path, plist).expect("write Info.plist");
    println!(
        "cargo:rustc-link-arg-bin=rift-terminal=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        path.display()
    );
}

fn windows() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("../../assets/icon/rift.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=no icon resource: only the GNU toolchain's windres is wired up");
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // "0.34.0" -> 0,34,0,0
    let mut parts: Vec<u16> = version
        .split(['.', '-'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    parts.resize(4, 0);
    let numeric = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc = out.join("rift.rc");
    let icon_path = icon.display().to_string().replace('\\', "/");
    fs::write(
        &rc,
        format!(
            r#"1 ICON "{icon_path}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Rift"
      VALUE "FileDescription", "Rift"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "rift-terminal"
      VALUE "OriginalFilename", "Rift.exe"
      VALUE "ProductName", "Rift"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        ),
    )
    .expect("write rift.rc");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            format!("{arch}-w64-mingw32-windres")
        }
    });
    let obj = out.join("rift-res.o");
    match Command::new(&windres)
        .arg(&rc)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj)
        .status()
    {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bin=rift-terminal={}", obj.display());
        }
        Ok(s) => println!("cargo:warning=no icon resource: {windres} failed ({s})"),
        Err(e) => println!("cargo:warning=no icon resource: {windres}: {e}"),
    }
}
