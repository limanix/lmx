//! Generates the gRPC code of `lmx.v1` with a vendored `protoc`, so builds need no system protoc.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure().compile_with_config(
        config,
        &["proto/lmx/v1/owner.proto"],
        &["proto"],
    )?;
    Ok(())
}
