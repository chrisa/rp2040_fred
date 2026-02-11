fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch == "arm" {
        println!("cargo:rustc-link-arg=-Tmemory.x");
        // embassy-rp provides this script to place `.boot2` into the BOOT2 memory region.
        println!("cargo:rustc-link-arg=-Tlink-rp.x");
    }
}
