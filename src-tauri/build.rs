fn main() {
    // Generate Phi Silica bindings from Windows App SDK winmd
    // This runs during build on any platform, generates bindings that are conditionally compiled
    let winmd_dir = std::path::Path::new(".winmd");

    if winmd_dir.exists()
        && winmd_dir.join("Microsoft.Windows.AI.Generative.winmd").exists()
        && winmd_dir.join("Windows.winmd").exists()
    {
        println!("cargo:rerun-if-changed=.winmd");

        // Collect all winmd files in the directory
        let mut args: Vec<&str> = Vec::new();

        // Add all winmd files as inputs
        let winmd_files: Vec<String> = std::fs::read_dir(winmd_dir)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()?.to_str()? == "winmd" {
                    Some(format!(".winmd/{}", entry.file_name().to_str()?))
                } else {
                    None
                }
            })
            .collect();

        // Build argument list
        for path in &winmd_files {
            args.push("--in");
            args.push(path);
        }

        // Output and filter
        args.push("--out");
        args.push("src/phi_silica_bindings.rs");
        args.push("--filter");
        args.push("Microsoft.Windows.AI.Generative");

        // Generate bindings
        let warnings = windows_bindgen::bindgen(&args);

        // Print any warnings
        for warning in warnings.iter() {
            println!("cargo:warning=windows-bindgen: {}", warning);
        }

        println!("cargo:warning=Generated Phi Silica bindings from {} winmd files", winmd_files.len());
    } else {
        println!(
            "cargo:warning=Phi Silica winmd files not found in .winmd/ directory - skipping binding generation"
        );
    }

    tauri_build::build()
}
