use apollo_compiler::Schema;
use std::path::{Path, PathBuf};

pub mod codegen;
mod sync;

/// Generates `OUT_DIR/necrassrs.rs` from `.graphql` files recursively discovered
/// under `schema_dir`, sorted by path. Symbolic-link entries are skipped.
pub fn build(schema_dir: impl AsRef<Path>) -> Result<(), BuildError> {
    let schema_dir = schema_dir.as_ref();
    println!("cargo::rerun-if-changed={}", schema_dir.display());
    let mut paths = schema_paths(schema_dir)?;

    // Keep generated output deterministic for the same inputs.
    paths.sort();

    let builder = paths
        .into_iter()
        .try_fold(Schema::builder(), |builder, path| {
            println!("cargo::rerun-if-changed={}", path.display());
            let source = std::fs::read_to_string(&path).map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?;

            Ok::<_, BuildError>(builder.parse(source, path))
        })?;

    let schema = builder
        .build()
        .map_err(|error| BuildError::Schema(error.errors))?
        .validate()
        .map_err(|error| BuildError::Schema(error.errors))?;
    let generated = codegen::generate(&schema).map_err(BuildError::Codegen)?;
    let out_dir = std::env::var("OUT_DIR").map_err(|source| BuildError::Environment {
        variable: "OUT_DIR",
        source,
    })?;
    let output = Path::new(&out_dir).join("necrassrs.rs");
    std::fs::write(&output, generated).map_err(|source| BuildError::Io {
        path: output,
        source,
    })?;

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").map_err(|source| BuildError::Environment {
            variable: "CARGO_MANIFEST_DIR",
            source,
        })?;

    let resolver_path = Path::new(&manifest_dir).join(
        "src/
    resolvers.rs",
    );

    sync::synchronize(&schema, &resolver_path)
}

fn schema_paths(directory: &Path) -> Result<Vec<PathBuf>, BuildError> {
    std::fs::read_dir(directory)
        .map_err(|source| BuildError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .try_fold(Vec::new(), |mut paths, entry| {
            let entry = entry.map_err(|source| BuildError::Io {
                path: directory.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            // Inspect the entry itself so symbolic links are not followed.
            let file_type = entry.file_type().map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?;

            if file_type.is_dir() {
                paths.extend(schema_paths(&path)?);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "graphql")
            {
                paths.push(path);
            }

            Ok(paths)
        })
}

#[derive(Debug)]
pub enum BuildError {
    Environment {
        variable: &'static str,
        source: std::env::VarError,
    },
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Schema(apollo_compiler::validation::DiagnosticList),
    Codegen(codegen::CodegenError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment { variable, source } => {
                write!(
                    formatter,
                    "Could not read environment variable {variable}: {source}"
                )
            }
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Schema(diagnostics) => std::fmt::Display::fmt(diagnostics, formatter),
            Self::Codegen(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Environment { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Schema(_) => None,
            Self::Codegen(error) => Some(error),
        }
    }
}
