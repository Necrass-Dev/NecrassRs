---
title: Organizing Resolver Files
description: Split resolver implementations across Rust modules while keeping SDL synchronization safe.
---

NecrassRs initially writes resolver declarations to `src/resolvers.rs`. As the
schema grows, you can move field resolver implementations into ordinary Rust
modules. Later builds discover those implementations through the module tree and
continue synchronizing them in place.

NecrassRs does not choose a file layout or move resolvers for you. You control the
module structure; the build only follows reachable modules from
`src/resolvers.rs`.

## Split a resolver into another file

Start with a generated `src/resolvers.rs`, then add a conventional Rust module:

```rust title="src/resolvers.rs"
pub struct Query;

mod query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}
```

Move the field resolver implementation into the matching module file. Because
the `Query` type remains in the parent module, change its receiver from
`self::Query` to `super::Query`:

```rust title="src/resolvers/query.rs"
impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::hello, C>
    for super::Query
{
    type Output = String;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::hello::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        Ok(format!("Hello, {}", args.name))
    }
}
```

The resulting layout is:

```text
src/
├── resolvers.rs
└── resolvers/
    └── query.rs
```

Run `cargo build` after moving the implementation. The build verifies that each
GraphQL field has at most one resolver implementation and updates the moved
implementation where it now lives.

You can also use `query/mod.rs`, nested external modules, inline modules, or a
mixture of them. Adjust `super::` paths as you would in any Rust module. New
resolver stubs are still appended to `src/resolvers.rs`; move them into your
chosen module after generation.

## Custom scalar modules

Custom scalars are application-owned types, so their editable implementations
live in the same resolver source tree. By default, the first custom scalar adds
`mod scalars;` and a public re-export to `src/resolvers.rs`, while its scaffold is
written to `src/resolvers/scalars.rs`. Generated contracts refer to the stable
`crate::resolvers::<Scalar>` path.

Synchronization appends new scalar scaffolds without rewriting edited scalar
representations or conversion bodies. It removes a stale scaffold and re-export
only when they remain untouched generated declarations. If an SDL scalar name
would collide with the internal `scalars` module, NecrassRs chooses a different
private module name without changing the public path.

See [Custom Scalars](/docs/custom-scalars/) for the generated scaffold and the
input and output conversion contract.

## Use conventional Rust modules

Resolver discovery recursively follows ordinary `mod <NAME>;` declarations and
inline `mod <NAME> { ... }` blocks, where `<NAME>` represents any Rust module
identifier. For each external module, it accepts exactly one of Rust's
conventional locations:

- `<NAME>.rs`
- `<NAME>/mod.rs`

The build rejects an ambiguous module when both locations exist. It also rejects
resolver module trees that use `#[path]`, conditional `#[cfg]` or `#[cfg_attr]`,
`include!`, macro-created modules, symbolic links, or missing module files. A
plain `#[cfg(test)] mod tests { ... }` is ignored during resolver discovery.

Files that are not reachable from `src/resolvers.rs` are not inspected or
modified.

## Read generated declarations

Generated argument types use `generated::types::<Object>::<field>::Args`.
Object modules, field modules, and argument fields preserve SDL case. Prefix
`self`, `Self`, `super`, and `crate` with one underscore, and prefix every SDL
name already starting with an underscore with one more underscore. Thus `self`
maps to `_self`, `_self` to `__self`, and `_` to `__`. This keeps distinct SDL
names distinct in Rust. Synchronization uses these mapped identities; a rename
is deletion plus addition, never an inferred move of an old resolver body.

Rust keywords use raw identifiers such as `r#type`. Generated declarations may
use qualified paths to avoid name collisions; equivalent path spelling can vary
without changing the public names above. Synchronization preserves resolver
bodies, so names shadowed inside application code may need manual adjustment.

## Understand source ownership

SDL owns the resolver contract. For a retained field, synchronization may update
the generated output type and other contract details while preserving the
`resolve` body, its parameter names, comments, and unrelated Rust code.

A resolver with an edited body, attribute, or comment is application-owned. When
a field is deleted or renamed, NecrassRs removes its old resolver only when it is
an untouched generated stub whose body is exactly `::core::unimplemented!()`.
Otherwise, the build stops before writing any resolver files. Remove or migrate
the implementation yourself, then build again.

When an SDL argument is removed, synchronization also stops if the retained body
directly reads that argument, such as `args.name`. Uses hidden inside macro input
cannot be interpreted reliably during source synchronization; Rust compilation
checks those after the generated argument type changes.

## Recover from synchronization errors

NecrassRs validates the complete set of resolver changes before writing. If it
detects an intervening edit from another process or editor, the build stops
instead of overwriting that edit.

If a write fails, the build attempts to restore files it already changed.
Rollback does not overwrite a file that changed again in the meantime.

A process termination can interrupt a multi-file update after only some files
were replaced; recovery of the whole set is not guaranteed. Inspect the resolver
files and run `cargo build` again to reconcile the remaining changes.
