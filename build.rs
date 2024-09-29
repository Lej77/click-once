fn main() {
    if std::env::var_os("CARGO_FEATURE_STD").is_none() {
        println!("cargo:rustc-link-arg=/ENTRY:_start");
        println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
    }
}
