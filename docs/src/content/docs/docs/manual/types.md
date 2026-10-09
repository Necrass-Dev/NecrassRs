---
title: Types
description: GraphQL-to-Rust type mappings, input presence, defaults, OneOf, and recursive inputs.
---

NecrassRs generates Rust argument and result types from your GraphQL SDL. This
page describes those types and the values a resolver receives. For a complete
web server example, follow the [Tutorial](/docs/tutorial/).

The generated API currently supports built-in scalar and enum results, ordinary
and OneOf input objects, owned Object results, Interface and Union results,
nullable and list forms, and recursive input and Object relationships. Custom
scalars, borrowed Object results, and generated mutation/subscription routing
remain unsupported.

## Rust mappings

| SDL                   | Rust                             |
| --------------------- | -------------------------------- |
| `Int!`                | `i32`                            |
| `Float!`              | `f64`                            |
| `String!`             | `String`                         |
| `Boolean!`            | `bool`                           |
| `ID!`                 | `necrassrs::Id`                  |
| Nullable input `Int`  | `GraphQLInput<i32>`              |
| Nullable result `Int` | `Option<i32>`                    |
| Input `[Int]`         | `GraphQLInput<Vec<Option<i32>>>` |
| Input `[Int]!`        | `Vec<Option<i32>>`               |
| Input `[Int!]`        | `GraphQLInput<Vec<i32>>`         |
| Input `[Int!]!`       | `Vec<i32>`                       |
| Result `[Int]`        | `Option<Vec<Option<i32>>>`       |
| Result `[Int]!`       | `Vec<Option<i32>>`               |
| Result `[Int!]`       | `Option<Vec<i32>>`               |
| Result `[Int!]!`      | `Vec<i32>`                       |

`GraphQLInput` is available from `necrassrs`. Generated field argument structs
live at `generated::types::<Object>::<field>::Args`; generated enums and input
objects live under `generated::types`.

List items have no undefined state. Input singleton coercion applies recursively:
for `[[Int]]`, the input `3` becomes `[[3]]`. Enum literals are unquoted, as in
`status: CLOSED`; JSON enum variables use strings, such as `{"status":"CLOSED"}`.
Quoted GraphQL enum literals and unknown enum members are rejected.

`Id` preserves supplied string contents, including leading zeros, and serializes
results as strings. Integer inputs are accepted. `Int` enforces signed 32-bit
bounds. JSON variable numbers with an empty fractional part are treated as
integers; a GraphQL floating-point literal remains a different literal kind.
Non-finite Float results produce execution errors; nullable list items preserve
valid siblings and report the failed item's index.

## Composite results

Applications provide the concrete Rust types returned for GraphQL Objects.
Interface and Union results use generated enums whose variants hold those concrete
types. Nullability and lists use the same `Option<T>` and `Vec<T>` wrappers as
scalar results.

| SDL result            | Resolver output example                                      |
| --------------------- | ------------------------------------------------------------ |
| `Operator!`           | `Operator`                                                   |
| `Operator`            | `Option<Operator>`                                           |
| `[Operator!]!`        | `Vec<Operator>`                                              |
| Interface `Person!`   | `generated::types::Person<Operator>`                         |
| Union `SearchResult!` | `generated::types::SearchResult<Operator, Company, Faction>` |

Every supported Query and Object field uses a
`necrassrs::Resolver<Field, Context>` implementation with an associated `Output`
type. The generated query-root resolver trait is an empty marker for the Rust
root type and Context. Recursive Object relationships are supported because each
selected field is resolved independently instead of nesting an infinitely sized
Rust type.
See the [Personnel Management System](/docs/personnel-management/) for a complete
Object, Interface, and Union example.

## GraphQLInput

GraphQL distinguishes an input that was not supplied from an input explicitly
set to `null`. "No value" is therefore ambiguous: the caller may have omitted
the input entirely, or supplied `null` as its value. A supplied non-null value
is a third state. Resolvers need to preserve all three meanings.

Nullable arguments and ordinary nullable input-object fields use the
`necrassrs::GraphQLInput<T>` enum:

```rust
pub enum GraphQLInput<T> {
    Undefined,
    Null,
    Value(T),
}
```

For a nullable input field `nickname: String` with no default:

| GraphQL input literal             | Meaning                                      | Rust value                                             |
| --------------------------------- | -------------------------------------------- | ------------------------------------------------------ |
| `{}`                              | The caller did not supply `nickname`.        | `GraphQLInput::Undefined`                              |
| `{ nickname: null }`              | The caller explicitly supplied a null value. | `GraphQLInput::Null`                                   |
| `{ nickname: "Tachibana Sheri" }` | The caller supplied a non-null string.       | `GraphQLInput::Value(String::from("Tachibana Sheri"))` |

GraphQL has no `undefined` input literal. `Undefined` represents absence: an
argument or input-object field is missing, or a variable has no entry in the JSON
variables object, with no applicable default supplying a value. A JSON entry
whose value is `null` is present and explicitly null.

An empty string, an empty list, zero, and `false` are supplied values, not omitted
inputs or nulls. When valid for the declared type, they become `Value(T)` just
like any other non-null value.

This matters when an application interprets an optional field as an update.
A resolver can treat `Undefined` as "leave the existing nickname unchanged",
`Null` as "clear the nickname", and `Value(name)` as "set the nickname to name".
These are application decisions; NecrassRs preserves the distinction so your
resolver can make them.

`Option<T>` only has `None` and `Some(T)`. Mapping both omission and explicit null
to `None` would lose the caller's intent. `GraphQLInput<T>` adds the separate
`Undefined` state while keeping `Null` and the supplied `T` in `Value(T)` distinct.

These states describe the value **after defaults and coercion**, not the original
request spelling. Omission can become `Value(T)` when a default supplies a value,
or `Null` when that default is null. Explicit null does not activate a default.
Non-null inputs reject null, and omission is an error when no applicable default
supplies a value. Nullable results use `Option<T>` because output values have no
undefined state.

### Input presence and defaults

Using the `Filter` and `statuses` definitions from the tutorial:

| Input                                  | Resolver value                                          |
| -------------------------------------- | ------------------------------------------------------- |
| Omit `filter`                          | `Value(Filter { ... })`, from the argument default `{}` |
| Omit `status` inside a supplied filter | `Value(Status::OPEN)`, from the field default           |
| Supply `status: null`                  | `Null`; the default does not replace explicit null      |
| Omit `limit`                           | `Undefined`, because it has no default                  |
| Supply `limit: null`                   | `Null`                                                  |
| Supply `limit: 5`                      | `Value(5)`                                              |

A variable default applies when that variable is absent. An explicit null variable
does not activate its default. An absent argument variable permits an applicable
argument default. Unknown input fields are rejected before resolver dispatch.

## OneOf inputs

The tutorial's `Locator` input is generated as an enum with `id(Id)` and
`name(String)` variants. The selected payload has no `GraphQLInput` wrapper.
Exactly one non-null entry is required: `{id: "001"}` is valid, while `{}`,
`{id: null}`, and `{id: "001", name: "Sheri"}` are invalid. OneOf schema fields
cannot be non-null or have defaults. A variable used directly as a member value
must have a compatible non-null variable type.

## Recursive inputs

In the tutorial's `Filter`, `next` uses `GraphQLInput<Box<Filter>>`; `children`
uses `GraphQLInput<Vec<Filter>>`. Lists already provide indirection. The generator
boxes nullable singular edges within recursive components, including mutual
recursion and OneOf payloads. Non-null and non-cyclic edges stay inline. Recursive
OneOf payloads use `Box<T>`, without another presence wrapper. GraphQL schema and
default-cycle validation are separate from this Rust layout analysis.
