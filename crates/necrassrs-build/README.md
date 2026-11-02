<div align="center">
  <h1><code>necrassrs-build</code></h1>
  <img src="https://raw.githubusercontent.com/Necrass-Dev/NecrassRs/main/logo/necrassrs.svg" alt="NecrassRs logo" width="180">
</div>

`necrassrs-build` validates GraphQL SDL during Cargo builds, generates Rust resolver contracts and dispatch code in `OUT_DIR`, and synchronizes resolver declarations in application source.

Add it as a build dependency and call it from `build.rs`:

```rust,no_run
fn main() -> Result<(), necrassrs_build::BuildError> {
    necrassrs_build::build("schema")
}
```

Building can update resolver source files. Review generated declarations after changing SDL. See the [NecrassRs tutorial](https://necrass.rs/docs/tutorial/) for the complete application setup and source ownership rules.

## License

Licensed under the [MIT License](LICENSE.md).
