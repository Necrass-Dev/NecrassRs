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

New declarations use ordinary names such as `Query` and `hello`. Rust keywords
keep their raw spelling, such as `r#type` and `r#gen`. This changes spelling, not
the SDL-to-Rust name mapping or GraphQL field names.

Standard types normally appear as `String`, `Vec<T>`, `Option<T>`, and
`Result<T, E>`. If a declaration, import, or generic parameter in that module
uses the same name, synchronization uses an absolute path such as
`::std::vec::Vec<T>`. A glob import also causes conservative qualification.
NecrassRs traits and types retain direct paths such as `necrassrs::Resolver`;
if `necrassrs` is shadowed, they use `::necrassrs::Resolver` instead.

Existing equivalent spelling is preserved. When new declarations introduce a
collision, synchronization can qualify the affected contract paths without
reformatting the file or rewriting resolver bodies. Existing aliases, custom
trait bounds, and names from glob imports are preserved rather than resolved.
Those application contracts and code inside a body may still need manual
adjustment if their own names become shadowed.

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

NecrassRs plans and validates all resolver changes before writing. Immediately
before replacing each file, it checks that the source still matches the version
used to make the plan. If another process or editor changed the file, the build
stops instead of overwriting that edit.

Each replacement is staged beside its destination. If a later replacement
fails, the build attempts to restore files it already changed. Rollback does not
overwrite a file that changed again in the meantime.

A process termination can interrupt a multi-file update after only some files
were replaced. Staging prevents an ordinary replacement from exposing partially
written source, but the whole set is not a durable transaction. Inspect the
resolver files and run `cargo build` again to reconcile the remaining changes.
