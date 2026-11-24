fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=icon.rc");
        println!("cargo:rerun-if-changed=icon.res");
        println!("cargo:rerun-if-changed=../../assets/icon.ico");

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let static_res = std::path::Path::new(&manifest_dir).join("icon.res");
        if static_res.exists() {
            println!("cargo:rustc-link-arg={}", static_res.display());
        }
    }
}
