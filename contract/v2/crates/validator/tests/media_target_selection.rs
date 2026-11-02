use serde_json::Value;

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../profiles/media-target-selection-v1/fixtures/receipts.json"
    ))
    .unwrap()
}

#[test]
fn valid_receipts_pass() {
    let f = fixtures();
    let v = &f["valid"];
    bgx_adapter_v2_validator::media_target_selection::validate_source(&v["sourceIdentity"]).unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_analysis(&v["analysisReceipt"]).unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_selection(
        &v["selection"],
        Some(&v["analysisReceipt"]),
    )
    .unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_selection(
        &v["selectionAbsent"],
        Some(&v["analysisReceipt"]),
    )
    .unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_tracking(&v["trackingReceipt"]).unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_freeze(&v["freeze"]).unwrap();
    bgx_adapter_v2_validator::media_target_selection::validate_refine_output(&v["refineOutput"]).unwrap();
}

#[test]
fn invalid_receipts_fail() {
    let f = fixtures();
    let inv = &f["invalid"];
    assert!(bgx_adapter_v2_validator::media_target_selection::validate_source(&inv["sourceWithPath"]).is_err());
    assert!(bgx_adapter_v2_validator::media_target_selection::validate_selection(
        &inv["selectionMissingShotChoice"],
        None
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::media_target_selection::validate_selection(
        &inv["selectionEmptyShots"],
        None
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::media_target_selection::validate_tracking(&inv["trackingNoReady"]).is_err());
}

#[test]
fn rejects_candidate_outside_analysis() {
    let f = fixtures();
    let mut sel = f["valid"]["selection"].clone();
    sel["shots"][0]["choice"]["candidateId"] = Value::String("c-unknown".into());
    let err = bgx_adapter_v2_validator::media_target_selection::validate_selection(
        &sel,
        Some(&f["valid"]["analysisReceipt"]),
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_INPUT");
}
