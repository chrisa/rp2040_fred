fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let defmt_enabled = std::env::var("CARGO_FEATURE_DEFMT_LOG").is_ok();
    if target_arch == "arm" {
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let out_path = std::path::PathBuf::from(out_dir);
        let memory_x = std::fs::read("memory.x").expect("failed to read memory.x");
        std::fs::write(out_path.join("memory.x"), memory_x).expect("failed to write memory.x");
        println!("cargo:rustc-link-search={}", out_path.display());

        // Core Cortex-M runtime script (vector table, Reset handler, sections).
        println!("cargo:rustc-link-arg=-Tlink.x");
        // embassy-rp provides this script to place `.boot2` into the BOOT2 memory region.
        println!("cargo:rustc-link-arg=-Tlink-rp.x");
        if defmt_enabled {
            println!("cargo:rustc-link-arg=-Tdefmt.x");
        }
    }
}
