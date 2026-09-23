use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn init_creates_project_at_explicit_path_and_name() {
    let directory = TestDirectory::new();
    let project = directory.0.join("my-api");
    let output = Command::new(env!("CARGO_BIN_EXE_necrass"))
        .arg("init")
        .arg(&project)
        .args(["--name", "custom-api"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        fs::read_to_string(project.join("Cargo.toml"))
            .unwrap()
            .contains("name = \"custom-api\""),
    );
    for path in [
        "README.md",
        "build.rs",
        "schema/schema.graphql",
        "src/main.rs",
        "src/generated.rs",
        "src/resolvers.rs",
    ] {
        assert!(project.join(path).is_file(), "missing {path}");
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "necrassrs-cli-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
