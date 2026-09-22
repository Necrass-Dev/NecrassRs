use apollo_compiler::{Schema, validation::Valid};
use std::path::Path;

use crate::BuildError;

pub(crate) fn synchronize(_schema: &Valid<Schema>, _path: &Path) -> Result<(), BuildError> {
    todo!()
}
