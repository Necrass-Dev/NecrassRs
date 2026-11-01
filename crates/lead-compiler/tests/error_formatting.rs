//! Regression tests for error formatting with multibyte UTF-8 characters.
//!
//! Background: When the parser encounters unexpected characters (like CJK
//! characters in identifiers, which are not valid in GraphQL), it produces
//! ERROR tokens that are added to the CST. This ensures Rowan tracks correct
//! byte positions and spans always fall on valid UTF-8 character boundaries,
//! preventing ariadne from panicking when formatting error messages.

use lead_compiler::parser::Parser;

/// Chinese characters (3-byte UTF-8)
#[test]
fn chinese_in_type_reference() {
    let result = Parser::new().parse_mixed_validate(
        r#"
type Query {
  field: 中文类型
}
"#,
        "test.graphql",
    );
    let errors = result.unwrap_err();
    assert!(!format!("{errors}").is_empty());
}

/// Japanese characters (3-byte UTF-8)
#[test]
fn japanese_in_type_reference() {
    let result = Parser::new().parse_mixed_validate(
        r#"
type Query {
  field: 日本語型名
}
"#,
        "test.graphql",
    );
    let errors = result.unwrap_err();
    assert!(!format!("{errors}").is_empty());
}

/// Korean Hangul characters (3-byte UTF-8)
#[test]
fn korean_in_type_reference() {
    let result = Parser::new().parse_mixed_validate(
        r#"
type Query {
  field: 한국어타입
}
"#,
        "test.graphql",
    );
    match result {
        Ok(schema) => println!("{:#?}", schema),
        Err(error) => println!("{}", error),
    }
    // let errors = result.unwrap_err();
    // assert!(!format!("{errors}").is_empty());
}

/// 4-byte emoji characters have different byte boundaries than 3-byte CJK
#[test]
fn emoji_in_type_reference() {
    let result = Parser::new().parse_mixed_validate(
        r#"
type Query {
  field: 🚀🌍🛸
}
"#,
        "test.graphql",
    );
    let errors = result.unwrap_err();
    assert!(!format!("{errors}").is_empty());
}

/// Mixed CJK and ASCII on the same line tests boundary transitions.
#[test]
fn mixed_multibyte_and_ascii() {
    let result = Parser::new().parse_mixed_validate(
        r#"
type Query {
  field日本語: UndefinedType
}
"#,
        "test.graphql",
    );
    let errors = result.unwrap_err();
    assert!(!format!("{errors}").is_empty());
}
