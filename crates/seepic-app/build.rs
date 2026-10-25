//! Embed the application icon in the Windows executable.
//!
//! Explorer, the taskbar, Alt-Tab and the window's title bar all take an
//! application's icon from the executable's resources; GPUI's own window icon
//! option is X11-only, so the resource is the only thing that reaches Windows.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/app-icon.rc");
    println!("cargo:rerun-if-changed=../../assets/app-icon.ico");

    #[cfg(windows)]
    embed_resource::compile(
        "../../assets/app-icon.rc",
        embed_resource::ParamsIncludeDirs(&["../../assets"]),
    )
    .manifest_optional()
    .expect("compile the Windows icon resource");
}
