use apollo_compiler::response::{
    GraphQLError, JsonMap,
    serde_json_bytes::{json, to_value},
};
use necrassrs::Response;

#[test]
fn request_error_response_omits_data() {
    let response = Response::request_error(GraphQLError {
        message: "Invalid request.".into(),
        locations: vec![],
        path: vec![],
        extensions: JsonMap::new(),
    });

    assert_eq!(
        to_value(response).unwrap(),
        json!({
            "errors": [{
                "message": "Invalid request."
            }]
        })
    );
}
