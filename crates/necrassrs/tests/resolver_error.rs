use necrassrs::ResolverError;

#[test]
fn resolver_error_preserves_message_and_extensions() {
    let error = ResolverError::new(r#"User "Unknown" was not found."#)
        .with_extension("code", "USER_NOT_FOUND");

    assert_eq!(error.message(), r#"User "Unknown" was not found."#);
    assert_eq!(
        error
            .extensions()
            .and_then(|extensions| extensions.get("code"))
            .and_then(|code| code.as_str()),
        Some("USER_NOT_FOUND")
    );
}

#[test]
fn resolver_error_omits_extensions_by_default() {
    let error = ResolverError::new("Something went wrong.");

    assert_eq!(error.message(), "Something went wrong.");
    assert!(error.extensions().is_none());
}
