use anyhow::{Result, Context};
use std::fs::File;
use std::path::Path;
use zip::write::FileOptions;
use zip::{ZipWriter, CompressionMethod};

pub fn create_zip<P: AsRef<Path>>(source_dir: P, zip_path: P) -> Result<()> {
    let source_dir = source_dir.as_ref();
    let zip_path = zip_path.as_ref();
    
    let file = File::create(zip_path)
        .with_context(|| format!("Failed to create zip file: {}", zip_path.display()))?;
    
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let walkdir = walkdir::WalkDir::new(source_dir);
    let it = walkdir.into_iter().filter_map(|e| e.ok());

    for entry in it {
        let path = entry.path();
        let name = path.strip_prefix(source_dir)
            .with_context(|| format!("Failed to strip prefix from path: {}", path.display()))?;

        if path.is_file() {
            zip.start_file(name.to_string_lossy(), options)
                .with_context(|| format!("Failed to start file in zip: {}", name.display()))?;
            
            let mut file = File::open(path)
                .with_context(|| format!("Failed to open file: {}", path.display()))?;
            
            std::io::copy(&mut file, &mut zip)
                .with_context(|| format!("Failed to copy file to zip: {}", path.display()))?;
        } else if !name.as_os_str().is_empty() {
            // Add directory entry
            zip.add_directory(name.to_string_lossy(), options)
                .with_context(|| format!("Failed to add directory to zip: {}", name.display()))?;
        }
    }

    zip.finish()
        .context("Failed to finish writing zip file")?;
    
    Ok(())
}