use serde_json::Value;

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../profiles/video-target-keyframes-v1/fixtures/receipts.json"
    ))
    .unwrap()
}

fn catalog() -> Value {
    serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json")).unwrap()
}

fn policy() -> Value {
    serde_json::from_str(include_str!("../../../protocol/schema-policy.json")).unwrap()
}

#[test]
fn valid_profile_documents_pass() {
    let f = fixtures();
    let v = &f["valid"];
    bgx_adapter_v2_validator::video_target_keyframes::validate_declaration(&v["declaration"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_selection(&v["selection"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_selection(&v["selectionWithBox"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&v["trackingInput"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_progress(&v["progress"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_receipt(&v["trackingReceiptReady"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_receipt(&v["trackingReceiptNotReady"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_frame_page(&v["framePage"]).unwrap();
    bgx_adapter_v2_validator::video_target_keyframes::validate_generation_freeze(
        &v["generationFreeze"],
        Some(&v["trackingReceiptReady"]),
    )
    .unwrap();
}

#[test]
fn invalid_profile_documents_fail() {
    let f = fixtures();
    let inv = &f["invalid"];
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_selection(
        &inv["selectionPointOutsideGeometry"]
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_selection(
        &inv["selectionNoTools"]
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(
        &inv["trackingMissingAnchor"]
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_receipt(
        &inv["receiptReadyWithoutCoverage"]
    )
    .is_err());
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_declaration(
        &inv["declarationEmptyTools"]
    )
    .is_err());
    let err = bgx_adapter_v2_validator::video_target_keyframes::validate_generation_freeze(
        &inv["freezeMaskMismatch"],
        Some(&f["valid"]["trackingReceiptReady"]),
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_INPUT");
}

#[test]
fn feature_catalog_lists_profile_and_rejects_unknown() {
    let catalog = catalog();
    let features = catalog["features"].as_array().unwrap();
    assert!(features.iter().any(|f| f["id"] == "video-target-keyframes-v1"));
    assert!(features.iter().any(|f| f["id"] == "media-target-selection-v1"));
    let known: std::collections::BTreeSet<_> = features
        .iter()
        .filter_map(|f| f["id"].as_str())
        .collect();
    assert!(!known.contains("not-a-real-feature-v9"));
    assert_eq!(
        bgx_adapter_v2_validator::video_target_keyframes::feature_id(),
        "video-target-keyframes-v1"
    );
}

#[test]
fn schema_policy_allows_x_target_tracking() {
    let policy = policy();
    let keywords = policy["supportedKeywords"].as_array().unwrap();
    assert!(keywords.iter().any(|k| k == "x-targetTracking"));
}

#[test]
fn relative_range_coverage_and_digest_are_closed() {
    let valid = fixtures()["valid"]["trackingInput"].clone();
    bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&valid).unwrap();

    let mut mismatch = valid.clone();
    mismatch["sourceIdentity"]["packageId"] = serde_json::json!("other.package");
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&mismatch).is_err());

    let mut range = valid.clone();
    range["range"]["endMs"] = serde_json::json!(4000);
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&range).is_err());

    let mut coverage = valid.clone();
    coverage["requestedCoverage"]["endMs"] = serde_json::json!(6000);
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&coverage).is_err());

    let mut time = valid.clone();
    time["selection"]["keyframes"][0]["timeMs"] = serde_json::json!(5000);
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_input(&time).is_err());

    let mut ready = fixtures()["valid"]["trackingReceiptReady"].clone();
    ready["uncomputedRanges"] = serde_json::json!([{"startMs": 0, "endMs": 100}]);
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_tracking_receipt(&ready).is_err());

    let index = serde_json::json!({
        "requestId": "req-1",
        "selectionDigest": "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "complete": false,
        "pages": [
            {"cursor": "a", "nextCursor": "b", "startMs": 0, "endMs": 2000, "mediaId": "page-a", "available": true},
            {"cursor": "b", "nextCursor": null, "startMs": 2000, "endMs": 5000, "mediaId": "page-b", "available": false}
        ]
    });
    bgx_adapter_v2_validator::video_target_keyframes::validate_frame_index(&index).unwrap();
    let mut cycle = index.clone();
    cycle["pages"][1]["nextCursor"] = serde_json::json!("a");
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_frame_index(&cycle).is_err());
    let mut dangling = index.clone();
    dangling["pages"][0]["nextCursor"] = serde_json::json!("missing");
    assert!(bgx_adapter_v2_validator::video_target_keyframes::validate_frame_index(&dangling).is_err());
}

#[test]
fn canonical_request_digest_bytes_are_stable() {
    let body = serde_json::json!({
        "selectionDigest": "abc",
        "requestedCoverage": {"startMs": 2500, "endMs": 5000},
        "range": {"startMs": 0, "endMs": 5000},
        "direction": "towardEnd",
        "anchorKeyframeId": "kf0"
    });
    let canonical = bgx_adapter_v2_validator::video_target_keyframes::canonical_json(&body);
    assert_eq!(
        canonical,
        r#"{"anchorKeyframeId":"kf0","direction":"towardEnd","range":{"endMs":5000,"startMs":0},"requestedCoverage":{"endMs":5000,"startMs":2500},"selectionDigest":"abc"}"#
    );
    let hex = bgx_adapter_v2_validator::video_target_keyframes::sha256_hex(canonical.as_bytes());
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn tracking_operation_must_accept_the_public_input_and_slot() {
    use bgx_adapter_v2_validator::video_target_keyframes::check_tracking_operation;
    let public = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/tracking.input.schema.json"}"#;
    let slot = |pointer: &str| serde_json::json!([{"role":"source","inputPointer":pointer,"mediaKind":"video","minCount":1,"maxCount":1}]);
    check_tracking_operation(public, &slot("/sourceIdentity/mediaId")).unwrap();
    let stale_slot = check_tracking_operation(public, &slot("/source")).unwrap_err();
    assert_eq!(stale_slot.path, "/source");
    let legacy = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,
        "required":["source","sourceRange","selection","direction","anchorKeyframeId","requestedCoverage","requestDigest","width","height","fps"],
        "properties":{"source":{"type":"object"},"sourceRange":{"type":"object"},"selection":{"type":"object"},"direction":{"type":"string"},
        "anchorKeyframeId":{"type":"string"},"requestedCoverage":{"type":"object"},"requestDigest":{"type":"string"},
        "width":{"type":"integer"},"height":{"type":"integer"},"fps":{"type":"integer"}}}"#;
    assert!(check_tracking_operation(legacy, &slot("/sourceIdentity/mediaId")).is_err());
}
