use apollo_compiler::{
    Name,
    response::{
        GraphQLError, JsonMap, ResponseDataPathSegment,
        serde_json_bytes::{json, to_value},
    },
};
use necrassrs::Response;

#[test]
fn execution_error_response_contains_null_data_and_path() {
    let response = Response::execution(
        None,
        vec![GraphQLError {
            message: "User not found.".into(),
            locations: vec![],
            path: vec![ResponseDataPathSegment::Field(
                Name::new_static("hello").unwrap(),
            )],
            extensions: JsonMap::new(),
        }],
    );

    assert_eq!(
        to_value(response).unwrap(),
        json!({
            "errors": [{
                "message": "User not found.",
                "path": ["hello"]
            }],
            "data": null
        })
    );
}
