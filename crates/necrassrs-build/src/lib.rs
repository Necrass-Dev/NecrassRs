use std::path::Path;

pub mod codegen;

pub fn build(_schema_dir: impl AsRef<Path>) -> Result<(), BuildError> {
    todo!()
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
