//! Gives the shared library a relocatable name, so a program finds it through
//! its own search path (rpath) instead of the folder it was built in:
//! `@rpath/libdiffyman_ffi.dylib` on macOS, the soname `libdiffyman_ffi.so`
//! on Linux.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos" | "ios") => {
            println!("cargo::rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libdiffyman_ffi.dylib");
        }
        Ok("linux" | "android" | "freebsd") => {
            println!("cargo::rustc-cdylib-link-arg=-Wl,-soname,libdiffyman_ffi.so");
        }
        _ => {}
    }
}
