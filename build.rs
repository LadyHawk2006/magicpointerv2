fn main() {
    // gtk4-layer-shell must be linked before libwayland-client in the final ELF
    println!("cargo:rustc-link-arg=-Wl,--no-as-needed");
    println!("cargo:rustc-link-lib=gtk4-layer-shell");
    println!("cargo:rustc-link-arg=-Wl,--as-needed");
}
