fn main() -> Result<(), Box<dyn std::error::Error>> {
    necrassrs_build::build("schema")?;
    Ok(())
}
