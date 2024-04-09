fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile(&["protos/lssdrpc.proto"], &["protos"])?;

    Ok(())
}
