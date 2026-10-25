//! Link fixes for the archive stack.
//!
//! `unrar_sys` compiles the official unrar C++ sources, which call into the
//! Windows registry and token APIs (`RegOpenKeyExW`, `RegQueryValueExW`,
//! `RegCloseKey`, `OpenProcessToken`). Its build script only links `powrprof`
//! and `shell32`, so `advapi32` has to come from somewhere.
//!
//! Binaries that also link GPUI happen to pull `advapi32` in transitively, which
//! is why this only shows up when building this crate's own test and bench
//! targets. Declaring it here keeps every target linkable on its own.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
