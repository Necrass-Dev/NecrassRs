fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    if let Err(error) = necrassrs_build::build("schema") {
        match error {
            necrassrs_build::BuildError::Codegen(error) => {
                let mut report = String::new();
                miette::NarratableReportHandler::new().render_report(&mut report, &error)?;
                eprintln!("{report}");
            }
            error => eprintln!("{error}"),
        }
        std::process::exit(1);
    }
    Ok(())
}
