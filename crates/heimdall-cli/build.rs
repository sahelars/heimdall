//! Embed an Info.plist in the macOS `heimdall` binary (SPEC §16).
//!
//! macOS privacy protection (TCC) guards Documents, Desktop, Downloads, iCloud
//! Drive, and removable and network volumes. An MCP client launches
//! `heimdall mcp` itself, so the vault is opened by this binary on its own
//! account — and a command-line tool with no Info.plist has no identity to be
//! granted access under and no words for macOS to put in the prompt. It is
//! refused (`Operation not permitted`), silently, which a client shows as a
//! server with no tools.
//!
//! A plist in the `__TEXT,__info_plist` section gives it both: a bundle
//! identifier that `codesign` signs it under, and the usage strings macOS shows
//! when it asks the user.

use std::path::PathBuf;

const IDENTIFIER: &str = "io.slarsen.heimdall.cli";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets the package version");
    let why = "Heimdall reads and writes the Markdown notes in the vaults you choose, for you and \
               for the AI apps you connect to it.";
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>{IDENTIFIER}</string>
	<key>CFBundleName</key>
	<string>heimdall</string>
	<key>CFBundleDisplayName</key>
	<string>Heimdall</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>NSDocumentsFolderUsageDescription</key>
	<string>{why}</string>
	<key>NSDesktopFolderUsageDescription</key>
	<string>{why}</string>
	<key>NSDownloadsFolderUsageDescription</key>
	<string>{why}</string>
	<key>NSRemovableVolumesUsageDescription</key>
	<string>{why}</string>
	<key>NSNetworkVolumesUsageDescription</key>
	<string>{why}</string>
	<key>NSFileProviderDomainUsageDescription</key>
	<string>{why}</string>
</dict>
</plist>
"#
    );

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("Info.plist");
    std::fs::write(&out, plist).expect("write Info.plist");
    println!(
        "cargo:rustc-link-arg-bin=heimdall=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        out.display()
    );
}
