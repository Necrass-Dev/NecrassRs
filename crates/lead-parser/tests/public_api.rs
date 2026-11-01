use lead_parser::{cst, cst::CstNode, Parser};

#[test]
fn parses_maintained_grammar_extensions() {
    let tree = Parser::new("\"operation\" query Example {}").parse();
    assert_eq!(tree.errors().len(), 0);
}

#[test]
fn decodes_valid_unicode_escapes_and_rejects_invalid_ones() {
    for (literal, expected) in [
        (r#""\uD83D\uDE00""#, "😀"),
        (r#""\u{1F600}""#, "😀"),
        (r#""\u{0000000041}""#, "A"),
    ] {
        let tree = Parser::new(&format!("{{ value(input: {literal}) }}")).parse();
        assert_eq!(tree.errors().len(), 0, "{literal}");
        let value = tree
            .document()
            .syntax()
            .descendants()
            .find_map(cst::StringValue::cast)
            .unwrap();
        assert_eq!(String::from(value), expected);
    }

    for literal in [r#""\uD800""#, r#""\u{}""#, r#""\u{110000}""#] {
        let tree = Parser::new(&format!("{{ value(input: {literal}) }}")).parse();
        assert!(tree.errors().len() > 0, "{literal}");
    }
}
