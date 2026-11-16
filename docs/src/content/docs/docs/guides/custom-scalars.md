---
title: Custom Scalars
description: Define application-owned scalar storage, input coercion, output serialization, and regeneration behavior.
---

GraphQL custom scalar declarations name a type but do not define its Rust
representation or conversion rules. NecrassRs generates an editable,
string-backed starting point for each custom scalar. Your application owns that
type and decides which GraphQL literals, JSON variable values, and response
values are valid.

## Declare a scalar in SDL

Declare the scalar and use it like any other GraphQL input or result type:

```graphql
scalar DateTime @specifiedBy(url: "https://scalars.graphql.org/andimarek/date-time.html")

type Query {
  echo(value: DateTime!): DateTime!
}
```

The optional `@specifiedBy` URL is exposed as `specifiedByURL` through
introspection. NecrassRs does not fetch the URL or infer validation from the
scalar name or specification.

## Build the scalar scaffold

Run `cargo build` after adding the scalar. By default, NecrassRs creates
`src/resolvers/scalars.rs` and re-exports the type from `src/resolvers.rs`:

```text
src/
├── resolvers.rs
└── resolvers/
    └── scalars.rs
```

The generated scaffold accepts GraphQL string literals and JSON string values
and serializes the stored string as JSON:

```rust title="src/resolvers/scalars.rs"
pub struct DateTime(::std::string::String);

impl ::necrassrs::Scalar for DateTime {
    fn parse_literal(
        value: &::necrassrs::ScalarLiteral,
    ) -> ::core::result::Result<Self, ::necrassrs::ScalarError> {
        match value {
            ::necrassrs::ScalarLiteral::String(value) => {
                ::core::result::Result::Ok(Self(value.to_owned()))
            }
            _ => ::core::result::Result::Err(::necrassrs::ScalarError::new(
                "expected a string literal",
            )),
        }
    }

    fn parse_value(
        value: &::necrassrs::JsonValue,
    ) -> ::core::result::Result<Self, ::necrassrs::ScalarError> {
        value
            .as_str()
            .map(|value| Self(value.to_owned()))
            .ok_or_else(|| ::necrassrs::ScalarError::new("expected a string value"))
    }

    fn serialize(
        &self,
    ) -> ::core::result::Result<::necrassrs::JsonValue, ::necrassrs::ScalarError> {
        ::core::result::Result::Ok(self.0.as_str().into())
    }
}
```

This scaffold is not a `DateTime` validator. Change its representation and
conversion bodies when the scalar needs format validation, a numeric transport
value, or another application type; [Parse date-time values](#parse-date-time-values)
replaces it with a validating implementation. Generated argument and result
contracts use the stable path `crate::resolvers::DateTime`.

## Define the conversion paths

The `necrassrs::Scalar` trait separates the three GraphQL boundaries:

| Method          | Receives                    | Used for                                             |
| --------------- | --------------------------- | ---------------------------------------------------- |
| `parse_literal` | `&necrassrs::ScalarLiteral` | GraphQL literals and GraphQL or SDL default literals |
| `parse_value`   | `&necrassrs::JsonValue`     | Values explicitly supplied in the JSON variables map |
| `serialize`     | `&self`                     | Resolver results written to the GraphQL response     |

Literal kinds are preserved until `parse_literal` runs. A string literal, enum
literal, integer literal, and floating-point literal therefore remain distinct.
Custom integer literals are not first restricted to the JSON number range, so a
scalar may match `ScalarLiteral::Int(value)` and retain an arbitrarily large
integer with `value.as_str()`.

Defaults keep their literal origin. This includes operation variable defaults
and SDL input-field defaults applied while coercing a JSON variable object. A
value explicitly present in the JSON variables map uses `parse_value`; a default
inserted because a value is absent uses `parse_literal`.

NecrassRs owns nullability, omission, and list coercion around the scalar. Your
implementation only converts one non-null scalar value at a time.

## Parse date-time values

This implementation stores an exact instant using the
[jiff](https://docs.rs/jiff/1/jiff/) crate and exchanges it as an RFC 3339
timestamp string. Add the dependency:

```sh
cargo add jiff@1
```

Replace the scaffold with:

```rust title="src/resolvers/scalars.rs"
use necrassrs::{JsonValue, Scalar, ScalarError, ScalarLiteral};

/// An exact instant, exchanged as an RFC 3339 timestamp string.
pub struct DateTime(pub jiff::Timestamp);

impl DateTime {
    fn parse(text: &str) -> Result<Self, ScalarError> {
        text.parse()
            .map(Self)
            .map_err(|error| ScalarError::new(format!("invalid DateTime: {error}")))
    }
}

impl Scalar for DateTime {
    fn parse_literal(value: &ScalarLiteral) -> Result<Self, ScalarError> {
        match value {
            ScalarLiteral::String(value) => Self::parse(value),
            _ => Err(ScalarError::new("expected a DateTime string literal")),
        }
    }

    fn parse_value(value: &JsonValue) -> Result<Self, ScalarError> {
        let text = value
            .as_str()
            .ok_or_else(|| ScalarError::new("expected a DateTime string value"))?;
        Self::parse(text)
    }

    fn serialize(&self) -> Result<JsonValue, ScalarError> {
        Ok(self.0.to_string().into())
    }
}
```

Both input paths share one parser, so a literal and a variable accept the same
strings. `jiff::Timestamp` requires a date, a time, and a UTC offset, and it
rejects calendar dates that do not exist. Later builds keep this file because
its type and conversion bodies are application code.

For the `echo` field above, the generated argument's `value` field and resolver
output are both the application-owned `DateTime` type, so the resolver returns
the already converted value:

```rust title="src/resolvers.rs"
impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::echo, C> for Query {
    type Output = crate::resolvers::DateTime;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::echo::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        Ok(args.value)
    }
}
```

| Input                         | Result                                 |
| ----------------------------- | -------------------------------------- |
| `"2026-11-16T09:30:00+09:00"` | `"2026-11-16T00:30:00Z"`               |
| `"2026-11-16T00:30:00.123Z"`  | `"2026-11-16T00:30:00.123Z"`           |
| `"2026-11-16"`                | Error: the time component is missing   |
| `"2026-11-16T09:30:00"`       | Error: the offset component is missing |
| `"2026-02-30T00:00:00Z"`      | Error: the date does not exist         |
| `1700000000`                  | Error: expected a string literal       |

A `Timestamp` represents an instant, not the offset the client wrote, so results
are serialized in UTC. Store a `jiff::Zoned` or the original offset instead when
the application must return the supplied offset.

## Handle conversion errors

Return `necrassrs::ScalarError::new(message)` when a conversion fails.

- Invalid custom scalar variables are rejected during variable coercion, before
  any field resolver runs.
- Invalid argument literals in queries and mutations become field execution
  errors with the applicable response path and null propagation. Invalid
  subscription root arguments fail source creation before a response stream is
  returned; see [Subscriptions](/docs/subscriptions/#handle-errors-and-termination).
- `serialize` failures are execution errors and follow the field's nullability.

Conversion methods must be deterministic and side-effect-free. NecrassRs does
not guarantee how many times a value is converted; a validated variable may be
converted again when the owned resolver argument is constructed.

## Regenerate safely

Later builds append scaffolds for new scalars while preserving edited scalar
types and conversion bodies. Removing or renaming a scalar deletes its old
scaffold and re-export only when they are still untouched generated declarations.
An edited stale scalar remains application code and must be removed manually.

NecrassRs normally uses the internal module name `scalars`. If an SDL type maps
to that Rust name, synchronization chooses a collision-free internal module name
while keeping the public scalar path under `crate::resolvers` unchanged. Scalar
names that collide with names such as `Result`, `Sync`, `std`, or `necrassrs`
also compile because generated scaffolds use qualified framework and standard
library paths.
