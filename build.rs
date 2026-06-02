//! Wires in any src/providers/<name>/ with a mod.rs, so a new provider needs zero
//! edits outside its own folder.

use std::{fs, path::Path};

fn main() {
    let providers_dir = Path::new("src/providers");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("providers_generated.rs");

    let mut names: Vec<String> = fs::read_dir(providers_dir)
        .expect("src/providers not found")
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.is_dir() && path.join("mod.rs").exists() {
                path.file_name()?.to_str().map(str::to_owned)
            } else {
                None
            }
        })
        .collect();
    names.sort();

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mods: String = names
        .iter()
        .map(|n| {
            format!(
                "#[path = \"{manifest}/src/providers/{n}/mod.rs\"]\npub mod {n};\n"
            )
        })
        .collect();
    let inits: String = names
        .iter()
        .map(|n| format!("    {n}::init(registry);\n"))
        .collect();

    let generated = format!(
        "{mods}\npub fn register_all(registry: &mut crate::providers::transform::ProviderRegistry) {{\n{inits}}}\n"
    );
    fs::write(&dest, generated).unwrap();

    println!("cargo:rerun-if-changed=src/providers");
}
