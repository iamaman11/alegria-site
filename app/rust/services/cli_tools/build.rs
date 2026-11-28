use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_dir = manifest_dir.join("../../../analytics_lab/proto");
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto_dir.join("analytics.proto")], &[proto_dir])?;
    Ok(())
}
