fn main() {
    // Only set windows subsystem for release builds
    #[cfg(all(windows, not(debug_assertions)))]
    {
        println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
        println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
    }
}