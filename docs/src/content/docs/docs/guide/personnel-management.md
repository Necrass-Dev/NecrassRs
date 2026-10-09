---
title: Personnel Management System
description: Build a GraphQL personnel system with Objects, Interfaces, Unions, recursive relationships, Context, and partial errors.
---

This tutorial builds a personnel management system on top of the server workflow
introduced in the [Tutorial](/docs/tutorial/). Start with a concrete `Operator`
object, then generalize the API with `Person` and `Organization` interfaces. The
finished schema adds recursive relationships, heterogeneous search, request
Context, and nullable error propagation.

The example keeps its data deliberately small. It contains four operator records:
Necrass, Reed, Reed the Flame Shadow, and Amiya.

:::caution[Unofficial example data]
Names, setting details, and related example data in this tutorial are based on
the game Arknights. Rights associated with Arknights belong to Yostar,
Hypergryph, and their respective rights holders. This example data is not
covered by the MIT License that applies to NecrassRs software. This tutorial is
unofficial fan content and is not approved, sponsored, or affiliated with those
rights holders.
:::

The tutorial uses names and short factual relationships only. It does not copy
game text, character art, logos, dialogue, or profile descriptions.

## What you will build

The final API supports these operations:

- Look up one concrete `Operator` by codename.
- Browse personnel through the `Person` interface.
- Browse a company and a faction through the `Organization` interface.
- Follow relationships from one person to another.
- Search across operators and organizations through a Union.
- Return a field-level authorization error without discarding the rest of the
  operator record.

Each section follows the same workflow:

1. Change `schema/schema.graphql`.
2. Run `cargo build` to validate the SDL and synchronize resolver declarations.
3. Keep each generated declaration and normally replace only its new
   `unimplemented!()` body. A section explicitly shows the full declaration when
   its Context type must also change.
4. Run the server and execute the section's query.

The code in this guide was verified in a temporary Axum project created by the
current `necrassrs-cli` and compiled against the current NecrassRs checkout.

## Create a fresh application

Install the CLI as described in the [installation guide](/docs/installation/),
then create a new Axum application:

```sh
necrass init personnel-system --framework axum
cd personnel-system
cargo build
```

The initialized application already contains the Axum route, embedded schema,
dispatcher, and a scalar greeting resolver. The next section replaces that
schema and lets the ordinary Cargo build synchronize the new declarations.

## 1. Return a concrete Object

Begin with a direct Object result. Replace `schema/schema.graphql` with:

```graphql ins={1-8}
type Operator {
  id: ID!
  codename: String!
}

type Query {
  operator(codename: String!): Operator
}
```

Run the build:

```sh
cargo build
```

The old scalar resolver is removed and `src/resolvers.rs` receives an
application-owned `Operator` representation and field-level resolver stubs.

Replace the generated unit struct with an identifier-bearing representation and
add the small in-memory data set:

```rust ins={3-26}
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct r#Operator {
    id: &'static str,
}

const OPERATOR_IDS: [&str; 4] = [
    "necrass",
    "reed",
    "reed-the-flame-shadow",
    "amiya",
];

fn operator_name(id: &str) -> Option<&'static str> {
    match id {
        "necrass" => Some("Necrass"),
        "reed" => Some("Reed"),
        "reed-the-flame-shadow" => Some("Reed the Flame Shadow"),
        "amiya" => Some("Amiya"),
        _ => None,
    }
}

fn operator(id: &'static str) -> self::r#Operator {
    self::r#Operator { id }
}
```

Keep the generated resolver declarations and replace their bodies as follows:

```rust ins={11-14,28,42-44}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Query::r#operator, C>
    for self::r#Query
{
    type Output = ::core::option::Option<self::r#Operator>;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::r#Query::r#operator::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        Ok(OPERATOR_IDS
            .into_iter()
            .find(|id| operator_name(id) == Some(args.codename.as_str()))
            .map(operator))
    }
}

impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#id, C>
    for self::r#Operator
{
    type Output = ::necrassrs::Id;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Operator::r#id::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        Ok(::necrassrs::Id::from(self.id))
    }
}

impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#codename, C>
    for self::r#Operator
{
    type Output = ::std::string::String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Operator::r#codename::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        operator_name(self.id)
            .map(str::to_owned)
            .ok_or_else(|| ::necrassrs::ResolverError::new("Unknown operator"))
    }
}
```

:::note[Why `resolve` instead of `operator`?]
Every supported field uses the same `Resolver<Field, Context>` contract,
including the scalar fields from the introductory tutorial. Here,
`fields::Query::operator` identifies the GraphQL field and the associated
`Output` type connects it to `Option<Operator>`. The method is always named
`resolve` because the field name lives in the trait parameter. Each field
therefore has its own implementation block. The empty `QueryResolver`
implementation only identifies the Rust query root and Context; it does not
contain field methods.
:::

Format and run the application:

```sh
cargo fmt
cargo run
```

Open GraphiQL at `http://127.0.0.1:3000/graphql` and run:

```graphql
{
  operator(codename: "Necrass") {
    id
    codename
  }
}
```

The response contains a concrete Object with independently resolved fields:

```json
{
  "data": {
    "operator": {
      "id": "necrass",
      "codename": "Necrass"
    }
  }
}
```

No Interface is involved yet. `Query.operator` returns `Operator`, and selection
of `id` and `codename` invokes the two `Operator` field resolvers.

## 2. Generalize personnel with an Interface

Now add a common contract for person-shaped records. Update the schema:

```graphql ins={1-4,6,8,14}
interface Person {
  id: ID!
  displayName: String!
}

type Operator implements Person {
  id: ID!
  displayName: String!
  codename: String!
}

type Query {
  operator(codename: String!): Operator
  personnel: [Person!]!
}
```

After `cargo build`, the new root resolver has this generated output contract:

```rust
type Output = ::std::vec::Vec<crate::generated::types::r#Person<self::r#Operator>>;
```

`Person` is generated as an enum over its possible concrete object types. Add
helpers that wrap each `Operator` in the corresponding variant:

```rust ins={1-7}
fn person(id: &'static str) -> crate::generated::types::r#Person<self::r#Operator> {
    crate::generated::types::r#Person::r#Operator(operator(id))
}

fn personnel() -> Vec<crate::generated::types::r#Person<self::r#Operator>> {
    OPERATOR_IDS.into_iter().map(person).collect()
}
```

Implement the new `displayName` field using the same stored identity:

```rust ins={11-13}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#displayName, C>
    for self::r#Operator
{
    type Output = ::std::string::String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Operator::r#displayName::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        operator_name(self.id)
            .map(str::to_owned)
            .ok_or_else(|| ::necrassrs::ResolverError::new("Unknown operator"))
    }
}
```

Then implement the generated `Query.personnel` stub:

```rust ins={11}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Query::r#personnel, C>
    for self::r#Query
{
    type Output = ::std::vec::Vec<crate::generated::types::r#Person<self::r#Operator>>;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Query::r#personnel::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        Ok(personnel())
    }
}
```

Stop the application from the previous section if it is still running, then
format and restart it:

```sh
cargo fmt
cargo run
```

Open GraphiQL at `http://127.0.0.1:3000/graphql`. Query common Interface fields
directly and use a fragment for the `Operator`-specific `codename` field:

```graphql
{
  personnel {
    __typename
    id
    displayName
    ... on Operator {
      codename
    }
  }
}
```

The list contains the four records and identifies their concrete type:

```json
{
  "data": {
    "personnel": [
      {
        "__typename": "Operator",
        "id": "necrass",
        "displayName": "Necrass",
        "codename": "Necrass"
      },
      { "__typename": "Operator", "id": "reed", "displayName": "Reed", "codename": "Reed" },
      {
        "__typename": "Operator",
        "id": "reed-the-flame-shadow",
        "displayName": "Reed the Flame Shadow",
        "codename": "Reed the Flame Shadow"
      },
      { "__typename": "Operator", "id": "amiya", "displayName": "Amiya", "codename": "Amiya" }
    ]
  }
}
```

This data set has one `Person` implementation, so the Interface does not yet
demonstrate a choice between object types. Its purpose here is to establish the
return contract used by organization membership and person-to-person
relationships. The next section introduces an Interface with two concrete
implementations.

## 3. Model organizations with multiple implementations

An operator may be associated with different kinds of organizations. Add an
`Organization` Interface, two concrete implementations, and the corresponding
fields:

```graphql ins={1-16}
interface Organization {
  id: ID!
  name: String!
}

type Company implements Organization {
  id: ID!
  name: String!
  personnel: [Person!]!
}

type Faction implements Organization {
  id: ID!
  name: String!
  members: [Person!]!
}
```

Add `affiliation` to `Operator`:

```graphql ins={5}
type Operator implements Person {
  id: ID!
  displayName: String!
  codename: String!
  affiliation: Organization
}
```

Add an organization list to `Query`:

```graphql ins={4}
type Query {
  operator(codename: String!): Operator
  personnel: [Person!]!
  organizations: [Organization!]!
}
```

Run `cargo build`. NecrassRs creates `Company` and `Faction` unit structures and
uses a generated enum for abstract results:

```rust
type Output =
    ::std::vec::Vec<crate::generated::types::r#Organization<self::r#Company, self::r#Faction>>;
```

Keep the generated unit structures. Replace the bodies for their leaf and
membership resolvers with the following expressions:

| Resolver            | Body                                          |
| ------------------- | --------------------------------------------- |
| `Company.id`        | `Ok(necrassrs::Id::from("rhodes-island"))`    |
| `Company.name`      | `Ok("Rhodes Island".to_owned())`              |
| `Company.personnel` | `Ok(personnel())`                             |
| `Faction.id`        | `Ok(necrassrs::Id::from("dublinn"))`          |
| `Faction.name`      | `Ok("Dublinn".to_owned())`                    |
| `Faction.members`   | `Ok(vec![person("necrass"), person("reed")])` |

In the generated `Query.organizations` implementation, keep the declaration and
replace only the `unimplemented!()` body. The completed resolver selects the
concrete variants explicitly:

```rust ins={12-15}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Query::r#organizations, C>
    for self::r#Query
{
    type Output =
        ::std::vec::Vec<crate::generated::types::r#Organization<self::r#Company, self::r#Faction>>;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Query::r#organizations::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        Ok(vec![
            crate::generated::types::r#Organization::r#Company(self::r#Company),
            crate::generated::types::r#Organization::r#Faction(self::r#Faction),
        ])
    }
}
```

Do the same for the generated `Operator.affiliation` implementation. The field
uses the same generated enum. This tutorial associates the Necrass record with
Dublinn and the other records with Rhodes Island:

```rust ins={13-21}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#affiliation, C>
    for self::r#Operator
{
    type Output = ::core::option::Option<
        crate::generated::types::r#Organization<self::r#Company, self::r#Faction>,
    >;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Operator::r#affiliation::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        let organization = match self.id {
            "necrass" => crate::generated::types::r#Organization::r#Faction(self::r#Faction),
            "reed" | "reed-the-flame-shadow" | "amiya" => {
                crate::generated::types::r#Organization::r#Company(self::r#Company)
            }
            _ => return Err(::necrassrs::ResolverError::new("Unknown operator")),
        };

        Ok(Some(organization))
    }
}
```

Query fields shared by the Interface, then select fields specific to each
implementation:

```graphql
{
  organizations {
    __typename
    id
    name
    ... on Company {
      personnel {
        displayName
      }
    }
    ... on Faction {
      members {
        displayName
      }
    }
  }
}
```

The `__typename` field shows which generated enum variant was returned:

```json
{
  "data": {
    "organizations": [
      {
        "__typename": "Company",
        "id": "rhodes-island",
        "name": "Rhodes Island",
        "personnel": [
          { "displayName": "Necrass" },
          { "displayName": "Reed" },
          { "displayName": "Reed the Flame Shadow" },
          { "displayName": "Amiya" }
        ]
      },
      {
        "__typename": "Faction",
        "id": "dublinn",
        "name": "Dublinn",
        "members": [{ "displayName": "Necrass" }, { "displayName": "Reed" }]
      }
    ]
  }
}
```

## 4. Follow recursive person relationships

Relationships lead from one person record to another. Add an enum, a relationship
Object, and a non-null list on `Operator`:

```graphql ins={1-10}
enum RelationshipKind {
  SIBLING
  ALTERNATE_FORM
  COOPERATES_WITH
}

type PersonRelationship {
  kind: RelationshipKind!
  person: Person!
}
```

```graphql ins={6}
type Operator implements Person {
  id: ID!
  displayName: String!
  codename: String!
  affiliation: Organization
  relationships: [PersonRelationship!]!
}
```

After `cargo build`, replace the generated `r#PersonRelationship` unit
structure with:

```rust ins={3-6}
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct r#PersonRelationship {
    kind: &'static str,
    person_id: &'static str,
}
```

`Operator` and the existing `operator(id)` helper remain unchanged. The
relationship kind and related-person identifier belong to
`r#PersonRelationship`, not `r#Operator`.

The operator resolver returns finite relationship records. Recursive GraphQL
selection does not require a recursively sized Rust value because each edge stores
only the next person's identifier:

```rust ins={11-43}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#relationships, C>
    for self::r#Operator
{
    type Output = ::std::vec::Vec<self::r#PersonRelationship>;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#Operator::r#relationships::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        let relationships = match self.id {
            "necrass" => vec![PersonRelationship {
                kind: "SIBLING",
                person_id: "reed",
            }],
            "reed" => vec![
                PersonRelationship {
                    kind: "SIBLING",
                    person_id: "necrass",
                },
                PersonRelationship {
                    kind: "ALTERNATE_FORM",
                    person_id: "reed-the-flame-shadow",
                },
            ],
            "reed-the-flame-shadow" => vec![
                PersonRelationship {
                    kind: "ALTERNATE_FORM",
                    person_id: "reed",
                },
                PersonRelationship {
                    kind: "COOPERATES_WITH",
                    person_id: "amiya",
                },
            ],
            "amiya" => vec![PersonRelationship {
                kind: "COOPERATES_WITH",
                person_id: "reed-the-flame-shadow",
            }],
            _ => return Err(::necrassrs::ResolverError::new("Unknown operator")),
        };

        Ok(relationships)
    }
}
```

Map the stored relationship name to the generated GraphQL enum:

```rust ins={11-20}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#PersonRelationship::r#kind, C>
    for self::r#PersonRelationship
{
    type Output = crate::generated::types::r#RelationshipKind;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#PersonRelationship::r#kind::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        match self.kind {
            "SIBLING" => Ok(crate::generated::types::r#RelationshipKind::r#SIBLING),
            "ALTERNATE_FORM" => {
                Ok(crate::generated::types::r#RelationshipKind::r#ALTERNATE_FORM)
            }
            "COOPERATES_WITH" => {
                Ok(crate::generated::types::r#RelationshipKind::r#COOPERATES_WITH)
            }
            _ => Err(::necrassrs::ResolverError::new("Unknown relationship")),
        }
    }
}
```

Return the related person through the `Person` Interface:

```rust ins={11}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#PersonRelationship::r#person, C>
    for self::r#PersonRelationship
{
    type Output = crate::generated::types::r#Person<self::r#Operator>;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::r#PersonRelationship::r#person::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        Ok(person(self.person_id))
    }
}
```

The query can follow the graph for as many statically written levels as it needs:

```graphql
{
  operator(codename: "Necrass") {
    codename
    relationships {
      kind
      person {
        displayName
        ... on Operator {
          relationships {
            kind
            person {
              displayName
            }
          }
        }
      }
    }
  }
}
```

GraphQL selection depth is finite even though the schema permits returning to
`Person` repeatedly. Each selected Object is completed one level at a time.

## 5. Search heterogeneous results with a Union

Interfaces expose fields shared by all implementations. A Union only identifies
which Object types may appear, so every object-specific selection uses a fragment.
Add the search result and Query field:

```graphql ins={1,7}
union SearchResult = Operator | Company | Faction

type Query {
  operator(codename: String!): Operator
  personnel: [Person!]!
  organizations: [Organization!]!
  search(text: String!): [SearchResult!]!
}
```

Run `cargo build`. The generated result contract lists every Union member:

```rust
type Output = ::std::vec::Vec<
    crate::generated::types::r#SearchResult<self::r#Operator, self::r#Company, self::r#Faction>,
>;
```

Implement a small case-insensitive search:

```rust ins={13-36}
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::r#Query::r#search, C>
    for self::r#Query
{
    type Output = ::std::vec::Vec<
        crate::generated::types::r#SearchResult<self::r#Operator, self::r#Company, self::r#Faction>,
    >;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::r#Query::r#search::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        let text = args.text.to_lowercase();
        let mut results = OPERATOR_IDS
            .into_iter()
            .filter(|id| {
                operator_name(id)
                    .is_some_and(|name| name.to_lowercase().contains(text.as_str()))
            })
            .map(|id| {
                crate::generated::types::r#SearchResult::r#Operator(operator(id))
            })
            .collect::<Vec<_>>();

        if "rhodes island".contains(text.as_str()) {
            results.push(crate::generated::types::r#SearchResult::r#Company(
                self::r#Company,
            ));
        }
        if "dublinn".contains(text.as_str()) {
            results.push(crate::generated::types::r#SearchResult::r#Faction(
                self::r#Faction,
            ));
        }

        Ok(results)
    }
}
```

Query each possible result using fragments:

```graphql
{
  search(text: "reed") {
    __typename
    ... on Operator {
      codename
    }
    ... on Company {
      name
    }
    ... on Faction {
      name
    }
  }
}
```

Both matching operator records are returned:

```json
{
  "data": {
    "search": [
      { "__typename": "Operator", "codename": "Reed" },
      { "__typename": "Operator", "codename": "Reed the Flame Shadow" }
    ]
  }
}
```

Returning `SearchResult::Operator`, `SearchResult::Company`, or
`SearchResult::Faction` is what establishes `__typename` at runtime. A value
outside those generated variants cannot be returned as this Union.

## 6. Protect nullable files with Context

The application constructs Context for every request. Use it to protect a
nullable personnel field while keeping the public operator data available.

Add the file Object and nullable field:

```graphql ins={1-4}
type PersonnelFile {
  title: String!
  content: String!
}
```

```graphql ins={7}
type Operator implements Person {
  id: ID!
  displayName: String!
  codename: String!
  affiliation: Organization
  relationships: [PersonRelationship!]!
  files: [PersonnelFile!]
}
```

Run `cargo build`, then replace the generated file representation and define the
request Context:

```rust ins={1-3,7-10}
pub struct RequestContext {
    pub can_read_confidential: bool,
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct r#PersonnelFile {
    title: &'static str,
    content: &'static str,
}
```

In `src/main.rs`, replace the unit Context in the GraphQL handler:

```rust ins={1-3}
let context = resolvers::RequestContext {
    can_read_confidential: false,
};
```

The other resolvers can remain generic over `C: Sync`. `Operator.files` is the
exception: replace its entire generated impl block, not only the
`unimplemented!()` body. The generated scaffold accepts a generic `C`, but this
resolver must name `RequestContext` before it can access
`can_read_confidential`. The highlighted declaration lines make that change:

```rust ins={1,8,11-22}
impl ::necrassrs::Resolver<crate::generated::fields::r#Operator::r#files, RequestContext>
    for self::r#Operator
{
    type Output = ::core::option::Option<::std::vec::Vec<self::r#PersonnelFile>>;

    async fn resolve(
        &self,
        context: &RequestContext,
        _args: crate::generated::types::r#Operator::r#files::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        if !context.can_read_confidential {
            return Err(
                ::necrassrs::ResolverError::new("Personnel files are confidential")
                    .with_extension("code", "FORBIDDEN"),
            );
        }

        Ok(Some(vec![self::r#PersonnelFile {
            title: "Personnel summary",
            content: operator_name(self.id)
                .ok_or_else(|| ::necrassrs::ResolverError::new("Unknown operator"))?,
        }]))
    }
}
```

Complete the two generated leaf resolvers with `Ok(self.title.to_owned())` and
`Ok(self.content.to_owned())` respectively.

With confidential access disabled, run:

```graphql
{
  operator(codename: "Necrass") {
    codename
    files {
      title
      content
    }
  }
}
```

The `files` field is nullable, so its resolver error does not discard the
successfully completed `codename` field:

```json
{
  "errors": [
    {
      "message": "Personnel files are confidential",
      "locations": [{ "line": 4, "column": 5 }],
      "path": ["operator", "files"],
      "extensions": { "code": "FORBIDDEN" }
    }
  ],
  "data": {
    "operator": {
      "codename": "Necrass",
      "files": null
    }
  }
}
```

Set `can_read_confidential` to `true` and send the same request to receive the
file data instead. Authentication and authorization policy remain application
responsibilities; Context only carries the request-specific decision into the
resolver.

## Final schema

After all sections, `schema/schema.graphql` contains:

```graphql
interface Person {
  id: ID!
  displayName: String!
}

interface Organization {
  id: ID!
  name: String!
}

type Operator implements Person {
  id: ID!
  displayName: String!
  codename: String!
  affiliation: Organization
  relationships: [PersonRelationship!]!
  files: [PersonnelFile!]
}

type Company implements Organization {
  id: ID!
  name: String!
  personnel: [Person!]!
}

type Faction implements Organization {
  id: ID!
  name: String!
  members: [Person!]!
}

type PersonRelationship {
  kind: RelationshipKind!
  person: Person!
}

type PersonnelFile {
  title: String!
  content: String!
}

enum RelationshipKind {
  SIBLING
  ALTERNATE_FORM
  COOPERATES_WITH
}

union SearchResult = Operator | Company | Faction

type Query {
  operator(codename: String!): Operator
  personnel: [Person!]!
  organizations: [Organization!]!
  search(text: String!): [SearchResult!]!
}
```

Run the final checks from the application directory:

```sh
cargo fmt --check
cargo check --locked
cargo run --locked
```

The completed application demonstrates the distinction between the three
composite result contracts:

| SDL result     | Resolver output                                   |
| -------------- | ------------------------------------------------- |
| `Operator`     | `Operator` or `Option<Operator>`                  |
| `Person`       | `types::Person<Operator>`                         |
| `Organization` | `types::Organization<Company, Faction>`           |
| `SearchResult` | `types::SearchResult<Operator, Company, Faction>` |

An Object value is returned directly. Interface and Union values use generated
enums to state which concrete Object is present. The same Object field resolvers
then complete the client's nested selection.
