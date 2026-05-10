use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_dir = manifest_dir.join("../../../contracts/proto");
    println!("cargo:rerun-if-changed={}", proto_dir.display());

    let read_api = proto_dir.join("read_api.proto");
    let rules = proto_dir.join("rules.proto");
    let condition = proto_dir.join("condition.proto");
    let sync = proto_dir.join("sync.proto");
    let temporal_payloads = proto_dir.join("temporal_payloads.proto");

    config.compile_protos(
        &[read_api, rules, condition, sync, temporal_payloads],
        &[proto_dir],
    )?;
    Ok(())
}
