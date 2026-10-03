use std::{fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/generated/core.ts");
    let generated = lenscribe_core::bindings::typescript();
    if std::env::args().any(|arg| arg == "--check") {
        if fs::read_to_string(&output)?.replace("\r\n", "\n") != generated {
            return Err("Frontend types are stale. Run bun run types:generate.".into());
        }
        println!("Frontend types match Rust.");
    } else {
        fs::create_dir_all(output.parent().unwrap())?;
        fs::write(output, generated)?;
        println!("Frontend types generated from Rust.");
    }
    Ok(())
}
