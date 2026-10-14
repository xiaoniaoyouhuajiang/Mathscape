fn main() {
    tauri_build::build();
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets = root.join("../../frontend/dist-inkpad");
    println!("cargo:rerun-if-changed={}", assets.display());
    let mut files = Vec::new();
    fn collect(
        root: &std::path::Path,
        dir: &std::path::Path,
        files: &mut Vec<(String, std::path::PathBuf)>,
    ) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect(root, &path, files);
                } else if path.is_file() {
                    files.push((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        path,
                    ));
                }
            }
        }
    }
    collect(&assets, &assets, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut source = String::from("pub(crate) static INKPAD_ASSETS: &[(&str, &[u8])] = &[\n");
    for (index, (name, path)) in files.into_iter().enumerate() {
        // Compile a stable snapshot: a later Vite rebuild may replace hashed files.
        let snapshot = output.join(format!("inkpad-asset-{index}"));
        std::fs::copy(path, &snapshot).expect("snapshot built InkPad asset");
        source.push_str(&format!(
            "({name:?}, include_bytes!({:?})),\n",
            snapshot.to_string_lossy()
        ));
    }
    source.push_str("];\n");
    std::fs::write(output.join("inkpad_assets.rs"), source).unwrap();
}
