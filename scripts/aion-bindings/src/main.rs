fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let checked = root.join("crates/wfdiag-native-phi/src/aion_bindings.rs");
    let check = std::env::args().any(|arg| arg == "--check");
    let generated = if check {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/aion_bindings.check.rs")
    } else {
        checked.clone()
    };
    windows_bindgen::bindgen([
        "--in",
        "default",
        root.join("crates/wfdiag-native-phi/metadata/AionInstructPreview.Text.winmd")
            .to_str()
            .unwrap(),
        "--out",
        generated.to_str().unwrap(),
        "--filter",
        "AionInstructPreview.Text",
        "--flat",
    ]);
    if check {
        assert_eq!(
            std::fs::read(&checked).unwrap(),
            std::fs::read(&generated).unwrap(),
            "Generated Aion bindings differ; regenerate from the pinned metadata"
        );
        println!("Aion bindings reproduce exactly");
    }
}
