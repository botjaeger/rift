//! macOS: embeds a minimal Info.plist in the executable's `__TEXT,__info_plist`
//! section so an unbundled run (`cargo run`) shows "Rift" in the menu bar
//! instead of the binary name. Inside `Rift.app` the bundle's own Info.plist
//! (packaging/make-app.sh) takes precedence.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
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
