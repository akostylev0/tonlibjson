// build trigger 1

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .type_attribute(
            "TvmEmulatorRunGetMethodResponse",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "TvmEmulatorSendExternalMessageResponse",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "TvmEmulatorSendInternalMessageResponse",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .type_attribute(
            "TransactionEmulatorEmulateResponse",
            "#[derive(serde::Deserialize, serde::Serialize)]",
        )
        .file_descriptor_set_path(out_dir.join("tvm_descriptor.bin"))
        .compile_protos(&["proto/tvm.proto"], &["proto"])?;

    Ok(())
}
