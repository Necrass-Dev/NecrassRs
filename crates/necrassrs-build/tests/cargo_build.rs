use std::{fs, path::Path, process::Command};

#[test]
fn cargo_build_compiles_generated_code_and_user_resolver() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = std::env::temp_dir().join(format!(
        "necrassrs-build-consumer-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir(&directory).unwrap();
    fs::create_dir(directory.join("src")).unwrap();
    fs::create_dir(directory.join("schema")).unwrap();

    let runtime = workspace.join("crates/necrassrs").canonicalize().unwrap();
    let build_library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"
                [package]
                name = "necrassrs-build-consumer"
                version = "0.0.0"
                edition = "2024"
                [workspace]
                [dependencies]
                necrassrs = {{ path = {runtime:?} }}
                [build-dependencies]
                necrassrs-build = {{ path = {build_library:?} }}
                [lints.rust]
                warnings = "deny"
            "#,
        ),
    )
    .unwrap();
    fs::copy(workspace.join("Cargo.lock"), directory.join("Cargo.lock")).unwrap();
    fs::write(
        directory.join("build.rs"),
        include_str!("fixtures/consumer/build.rs"),
    )
    .unwrap();
    fs::write(
        directory.join("schema/schema.graphql"),
        include_str!("fixtures/consumer/schema.graphql"),
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        include_str!("fixtures/consumer/main.rs"),
    )
    .unwrap();

    let build = Command::new(env!("CARGO"))
        .current_dir(&directory)
        .args(["build", "--offline", "--target-dir"])
        .arg(workspace.join("target/cargo-consumer"))
        .output();
    fs::remove_dir_all(&directory).unwrap();
    let build = build.expect("Cargo must be available");
    assert!(
        build.status.success(),
        "consumer cargo build failed:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );
}
