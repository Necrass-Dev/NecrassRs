use apollo_compiler::Schema;
use std::path::{Path, PathBuf};

pub mod codegen;
mod sync;

/// Generates `OUT_DIR/necrassrs.rs` from `.graphql` files recursively discovered
/// under `schema_dir`, sorted by path. Symbolic-link entries are skipped.
/// Creates and synchronizes editable query resolvers in `src/resolvers.rs`.
pub fn build(schema_dir: impl AsRef<Path>) -> Result<(), BuildError> {
    let out_dir = std::env::var("OUT_DIR").map_err(|source| BuildError::Environment {
        variable: "OUT_DIR",
        source,
    })?;
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").map_err(|source| BuildError::Environment {
            variable: "CARGO_MANIFEST_DIR",
            source,
        })?;
    build_at(
        schema_dir.as_ref(),
        Path::new(&out_dir),
        Path::new(&manifest_dir),
    )
}

fn build_at(schema_dir: &Path, out_dir: &Path, manifest_dir: &Path) -> Result<(), BuildError> {
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
    let output = out_dir.join("necrassrs.rs");
    std::fs::write(&output, generated).map_err(|source| BuildError::Io {
        path: output,
        source,
    })?;

    let resolver_path = manifest_dir.join("src/resolvers.rs");

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
    ResolverSource {
        path: PathBuf,
        source: syn::Error,
    },
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
            Self::ResolverSource { path, source } => {
                write!(formatter, "{}: {source}", path.display())
            }
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
            Self::ResolverSource { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BuildDir(PathBuf);

    impl BuildDir {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "necrassrs-build-unit-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn build(&self) -> Result<(), BuildError> {
            super::build_at(&self.0.join("schema"), &self.0.join("out"), &self.0)
        }
    }

    impl Drop for BuildDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn discovers_nested_graphql_files_without_following_symlinks() {
        let directory = BuildDir::new();
        let schema = directory.0.join("schema");
        std::fs::create_dir_all(schema.join("nested")).unwrap();
        std::fs::write(schema.join("b.graphql"), "type Query { b: String! }").unwrap();
        std::fs::write(
            schema.join("nested/a.graphql"),
            "extend type Query { a: String! }",
        )
        .unwrap();
        std::fs::write(schema.join("ignore.txt"), "ignored").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(schema.join("b.graphql"), schema.join("link.graphql"))
                .unwrap();
            std::os::unix::fs::symlink(schema.join("nested"), schema.join("linked_dir")).unwrap();
        }

        let mut paths = schema_paths(&schema).unwrap();
        paths.sort();
        assert_eq!(
            paths,
            [schema.join("b.graphql"), schema.join("nested/a.graphql")]
        );
        assert!(matches!(
            schema_paths(&schema.join("missing")),
            Err(BuildError::Io { .. })
        ));
    }

    #[test]
    fn build_generates_contract_and_editable_resolvers() {
        let directory = BuildDir::new();
        std::fs::create_dir_all(directory.0.join("schema/nested")).unwrap();
        std::fs::create_dir(directory.0.join("out")).unwrap();
        std::fs::create_dir(directory.0.join("src")).unwrap();
        std::fs::write(
            directory.0.join("schema/query.graphql"),
            "type Query { hello: String! }",
        )
        .unwrap();
        std::fs::write(
            directory.0.join("schema/nested/extra.graphql"),
            "extend type Query { extra: String! }",
        )
        .unwrap();

        directory.build().unwrap();
        let generated = std::fs::read_to_string(directory.0.join("out/necrassrs.rs")).unwrap();
        let resolvers = std::fs::read_to_string(directory.0.join("src/resolvers.rs")).unwrap();
        assert!(generated.contains("hello"));
        assert!(generated.contains("extra"));
        assert!(resolvers.contains("async fn r#hello"));
        assert!(resolvers.contains("async fn r#extra"));

        directory.build().unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.0.join("src/resolvers.rs")).unwrap(),
            resolvers
        );
    }

    #[test]
    fn validation_and_codegen_errors_leave_existing_source_untouched() {
        let directory = BuildDir::new();
        std::fs::create_dir(directory.0.join("schema")).unwrap();
        std::fs::create_dir(directory.0.join("out")).unwrap();
        std::fs::create_dir(directory.0.join("src")).unwrap();
        let sdl = directory.0.join("schema/query.graphql");
        let resolver = directory.0.join("src/resolvers.rs");
        std::fs::write(&resolver, "user code").unwrap();
        for (input, expected) in [
            ("type Query { hello(: String!): String! }", "schema"),
            ("type Query { count: Int! }", "codegen"),
        ] {
            std::fs::write(&sdl, input).unwrap();
            let error = directory.build().unwrap_err();
            assert!(
                matches!(
                    (&error, expected),
                    (BuildError::Schema(_), "schema") | (BuildError::Codegen(_), "codegen")
                ),
                "{error}"
            );
            assert_eq!(std::fs::read_to_string(&resolver).unwrap(), "user code");
        }
    }

    #[test]
    fn reports_missing_inputs_and_output_destinations() {
        let directory = BuildDir::new();
        assert!(matches!(directory.build(), Err(BuildError::Io { .. })));

        std::fs::create_dir(directory.0.join("schema")).unwrap();
        std::fs::write(
            directory.0.join("schema/query.graphql"),
            "type Query { hello: String! }",
        )
        .unwrap();
        assert!(matches!(directory.build(), Err(BuildError::Io { .. })));

        std::fs::create_dir(directory.0.join("out")).unwrap();
        assert!(matches!(directory.build(), Err(BuildError::Io { .. })));
    }

    #[test]
    fn build_errors_expose_their_causes_and_locations() {
        use std::error::Error;

        let path = PathBuf::from("schema/query.graphql");
        let errors = [
            BuildError::Environment {
                variable: "OUT_DIR",
                source: std::env::VarError::NotPresent,
            },
            BuildError::Io {
                path: path.clone(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            },
            BuildError::ResolverSource {
                path,
                source: syn::Error::new(proc_macro2::Span::call_site(), "invalid resolver"),
            },
        ];
        for (error, expected) in
            errors
                .into_iter()
                .zip(["OUT_DIR", "schema/query.graphql", "invalid resolver"])
        {
            assert!(error.to_string().contains(expected));
            assert!(error.source().is_some());
        }

        let invalid = Schema::parse_and_validate(
            "type Query { hello(: String!): String! }",
            "schema.graphql",
        )
        .unwrap_err();
        let schema_error = BuildError::Schema(invalid.errors);
        assert!(!schema_error.to_string().is_empty());
        assert!(schema_error.source().is_none());

        let valid =
            Schema::parse_and_validate("type Query { count: Int! }", "schema.graphql").unwrap();
        let codegen_error = BuildError::Codegen(codegen::generate(&valid).unwrap_err());
        assert!(
            codegen_error
                .to_string()
                .contains("Unsupported return type")
        );
        assert!(codegen_error.source().is_some());
    }
}
