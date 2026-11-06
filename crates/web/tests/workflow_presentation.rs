#[test]
fn workflow_presentation_shared_golden_bytes() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../client/src/testdata/workflow-presentation.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let request = case["request"].to_string();
        let expected = farcooler_client::workflow_presentation::json(&request);
        let actual = farcooler_web::workflow::presentation(&request);
        assert_eq!(actual, expected, "{}", case["name"]);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), case["response"]);
    }
    assert_eq!(farcooler_web::workflow::presentation("{}"),
        farcooler_client::workflow_presentation::json("{}"));
}
