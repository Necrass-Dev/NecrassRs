use std::{
    ffi::{OsStr, OsString},
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "Usage: necrass init [PATH] [--name NAME]";

fn prepare_target(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() && fs::read_dir(path)?.next().is_none() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("target is not an empty directory: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    Ok(())
}

fn create_package(path: &Path, name: &str) -> io::Result<()> {
    prepare_target(path)?;
    fs::create_dir(path.join("src"))?;
    fs::create_dir(path.join("schema"))?;
    for (relative, contents) in [
        (
            "Cargo.toml",
            include_str!("../templates/manifest.toml").replace("{{name}}", name),
        ),
        (
            "README.md",
            include_str!("../templates/README.md").to_owned(),
        ),
        ("build.rs", include_str!("../templates/build.rs").to_owned()),
        (
            "schema/schema.graphql",
            include_str!("../templates/schema.graphql").to_owned(),
        ),
        (
            "src/main.rs",
            include_str!("../templates/main.rs").to_owned(),
        ),
        (
            "src/generated.rs",
            include_str!("../templates/generated.rs").to_owned(),
        ),
        (
            "src/resolvers.rs",
            include_str!("../templates/resolvers.rs").to_owned(),
        ),
    ] {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(relative))?
            .write_all(contents.as_bytes())?;
    }
    Ok(())
}

fn parse_args(
    mut args: impl Iterator<Item = OsString>,
    current_dir: &Path,
) -> Result<(PathBuf, String), String> {
    if args.next().as_deref() != Some(OsStr::new("init")) {
        return Err(USAGE.into());
    }
    let mut path = None;
    let mut name = None;
    while let Some(arg) = args.next() {
        if arg == "--name" {
            if name.is_some() {
                return Err(USAGE.into());
            }
            name = Some(
                args.next()
                    .ok_or(USAGE)?
                    .into_string()
                    .map_err(|_| "Project name must be valid UTF-8.")?,
            );
        } else if arg.to_string_lossy().starts_with('-') || path.is_some() {
            return Err(USAGE.into());
        } else {
            path = Some(PathBuf::from(arg));
        }
    }
    let path = path.unwrap_or_else(|| current_dir.to_path_buf());
    let path = if path == Path::new(".") {
        current_dir.to_path_buf()
    } else {
        path
    };
    let name = name
        .or_else(|| path.file_name()?.to_str().map(str::to_owned))
        .ok_or("Could not derive a project name from the target directory.")?;
    let mut characters = name.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Project name must start with a letter or underscore and contain only ASCII letters, digits, hyphens, or underscores.".into());
    }
    Ok((path, name))
}

fn run() -> Result<(), String> {
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let (path, name) = parse_args(std::env::args_os().skip(1), &current_dir)?;
    create_package(&path, &name).map_err(|error| {
        format!(
            "Could not initialize project at {}: {error}",
            path.display()
        )
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_path_and_name_defaults_and_override() {
        let cwd = Path::new("/tmp/current-project");
        for (args, path, name) in [
            (vec!["init"], cwd.to_path_buf(), "current-project"),
            (vec!["init", "."], cwd.to_path_buf(), "current-project"),
            (
                vec!["init", "next-project"],
                PathBuf::from("next-project"),
                "next-project",
            ),
            (
                vec!["init", "next-project", "--name", "custom"],
                PathBuf::from("next-project"),
                "custom",
            ),
        ] {
            let parsed = parse_args(args.into_iter().map(OsString::from), cwd).unwrap();
            assert_eq!(parsed, (path, name.to_owned()));
        }
    }

    #[test]
    fn rejects_invalid_invocations() {
        for args in [
            vec![],
            vec!["unknown"],
            vec!["init", "--name"],
            vec!["init", "first", "second"],
            vec!["init", "--name", "one", "--name", "two"],
            vec!["init", "--name", "bad name"],
        ] {
            assert!(parse_args(args.into_iter().map(OsString::from), Path::new("/tmp")).is_err());
        }
    }

    #[test]
    fn prepares_only_missing_or_empty_targets() {
        let directory = TestDirectory::new();
        let empty = directory.0.join("empty");
        prepare_target(&empty).unwrap();
        prepare_target(&empty).unwrap();

        let file = directory.0.join("file");
        fs::write(&file, "keep this").unwrap();
        let nonempty = directory.0.join("nonempty");
        fs::create_dir(&nonempty).unwrap();
        fs::write(nonempty.join("keep.txt"), "keep this").unwrap();
        for target in [&file, &nonempty] {
            assert_eq!(
                prepare_target(target).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep this");
        assert_eq!(
            fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
            "keep this"
        );

        let invalid_parent = file.join("child");
        assert!(prepare_target(&invalid_parent).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep this");

        #[cfg(unix)]
        for (index, destination) in [empty, directory.0.join("missing")].iter().enumerate() {
            let link = directory.0.join(format!("link-{index}"));
            std::os::unix::fs::symlink(destination, &link).unwrap();
            assert_eq!(
                prepare_target(&link).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read_link(&link).unwrap(), destination.as_path());
        }
    }

    #[test]
    fn creating_inside_a_workspace_does_not_edit_its_manifest() {
        let directory = TestDirectory::new();
        let parent_manifest = directory.0.join("Cargo.toml");
        let original = "[workspace]\nmembers = []\n";
        fs::write(&parent_manifest, original).unwrap();

        let child = directory.0.join("child");
        create_package(&child, "child").unwrap();

        assert_eq!(fs::read_to_string(parent_manifest).unwrap(), original);
        assert!(
            fs::read_to_string(child.join("Cargo.toml"))
                .unwrap()
                .contains("[workspace]")
        );
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "necrassrs-cli-unit-{}-{}",
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
}
