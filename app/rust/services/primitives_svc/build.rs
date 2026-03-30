use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto = manifest_dir.join("../../../contracts/proto/primitives.proto");
    tonic_build::compile_protos(proto)?;
    Ok(())
}
