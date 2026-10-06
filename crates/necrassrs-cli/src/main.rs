use std::{
    fs, io,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use dialoguer::{Input, Select};

#[derive(Parser)]
#[command(
    name = "necrass",
    version,
    about = "Create a NecrassRs GraphQL project"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a project in a new or empty directory.
    Init(InitArgs),
}

#[derive(clap::Args)]
struct InitArgs {
    /// Target directory (defaults to the current directory).
    path: Option<PathBuf>,
    /// Cargo package name (defaults to the directory name).
    #[arg(long, value_parser = validate_name)]
    name: Option<String>,
    /// HTTP framework for the generated server.
    #[arg(long, value_enum, default_value = "axum")]
    framework: Framework,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Framework {
    Axum,
    Actix,
}

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

fn create_package(path: &Path, name: &str, framework: Framework) -> io::Result<()> {
    let (manifest, main) = match framework {
        Framework::Axum => (
            include_str!("../templates/axum/manifest.toml"),
            include_str!("../templates/axum/main.rs"),
        ),
        Framework::Actix => (
            include_str!("../templates/actix/manifest.toml"),
            include_str!("../templates/actix/main.rs"),
        ),
    };
    prepare_target(path)?;
    fs::create_dir(path.join("src"))?;
    fs::create_dir(path.join("schema"))?;
    for (relative, contents) in [
        ("Cargo.toml", manifest.replace("{{name}}", name)),
        (
            "README.md",
            include_str!("../templates/shared/README.md").to_owned(),
        ),
        (
            "build.rs",
            include_str!("../templates/shared/build.rs").to_owned(),
        ),
        (
            "schema/schema.graphql",
            include_str!("../templates/shared/schema.graphql").to_owned(),
        ),
        ("src/main.rs", main.to_owned()),
        (
            "src/generated.rs",
            include_str!("../templates/shared/generated.rs").to_owned(),
        ),
        (
            "src/resolvers.rs",
            include_str!("../templates/shared/resolvers.rs").to_owned(),
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

fn resolve_init(
    init: InitArgs,
    current_dir: &Path,
) -> Result<(PathBuf, String, Framework), String> {
    let path = init.path.unwrap_or_else(|| current_dir.to_path_buf());
    let path = if path == Path::new(".") {
        current_dir.to_path_buf()
    } else {
        path
    };
    let name = init
        .name
        .or_else(|| path.file_name()?.to_str().map(str::to_owned))
        .ok_or("Could not derive a project name from the target directory.")?;
    Ok((path, validate_name(&name)?, init.framework))
}

fn validate_name(name: &str) -> Result<String, String> {
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
    Ok(name.to_owned())
}

fn interactive_init(current_dir: &Path) -> Result<InitArgs, String> {
    let path: String = Input::new()
        .with_prompt("Project directory")
        .default("my-api".into())
        .interact_text()
        .map_err(|error| error.to_string())?;
    let path = PathBuf::from(path);
    let name_path = if path == Path::new(".") {
        current_dir
    } else {
        &path
    };
    let mut prompt = Input::<String>::new()
        .with_prompt("Package name")
        .validate_with(|value: &String| validate_name(value).map(|_| ()));
    if let Some(name) = name_path.file_name().and_then(|name| name.to_str()) {
        prompt = prompt.default(name.to_owned());
    }
    let name = prompt.interact_text().map_err(|error| error.to_string())?;
    let frameworks = Framework::value_variants();
    let choices: Vec<_> = frameworks
        .iter()
        .map(|framework| framework.to_possible_value().unwrap().get_name().to_owned())
        .collect();
    let selected = Select::new()
        .with_prompt("HTTP framework")
        .items(&choices)
        .default(0)
        .interact_opt()
        .map_err(|error| error.to_string())?
        .ok_or("Initialization cancelled.")?;
    Ok(InitArgs {
        path: Some(path),
        name: Some(name),
        framework: frameworks[selected],
    })
}

fn run(cli: Cli) -> Result<(), String> {
    if cli.command.is_none() && !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        Cli::command()
            .print_help()
            .map_err(|error| error.to_string())?;
        return Err("Interactive initialization requires a terminal. Use necrass init [PATH] [--name NAME] [--framework axum|actix].".into());
    }
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let init = match cli.command {
        Some(Commands::Init(init)) => init,
        None => interactive_init(&current_dir)?,
    };
    let (path, name, framework) = resolve_init(init, &current_dir)?;
    create_package(&path, &name, framework).map_err(|error| {
        format!(
            "Could not initialize project at {}: {error}",
            path.display()
        )
    })
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
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
            let cli = Cli::try_parse_from(std::iter::once("necrass").chain(args)).unwrap();
            let Some(Commands::Init(init)) = cli.command else {
                panic!("expected init")
            };
            assert_eq!(
                resolve_init(init, cwd).unwrap(),
                (path, name.to_owned(), Framework::Axum)
            );
        }
    }

    #[test]
    fn rejects_invalid_invocations() {
        for args in [
            vec!["unknown"],
            vec!["init", "--name"],
            vec!["init", "first", "second"],
            vec!["init", "--name", "one", "--name", "two"],
            vec!["init", "--name", "bad name"],
            vec!["init", "--framework", "unknown"],
            vec!["init", "--unknown"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("necrass").chain(args)).is_err());
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
        create_package(&child, "child", Framework::Axum).unwrap();

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
