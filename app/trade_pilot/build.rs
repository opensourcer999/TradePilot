//! Embeds real Win32 VERSIONINFO resources into the built .exe - FileVersion/ProductVersion are
//! filled in automatically by `WindowsResource::new()` from Cargo.toml's `package.version`;
//! FileDescription/ProductName default to the raw crate name ("`trade_pilot`"), overridden below to
//! the app's actual display name. Without this, Explorer's Properties > Details tab is blank.
//! Windows-only: other targets have no PE resources to embed this into.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=TRADEPILOT_BUILD_INVOCATION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set("FileDescription", "TradePilot").set("ProductName", "TradePilot").set_icon("assets/icon.ico");
        res.compile()?;
    }
    Ok(())
}
