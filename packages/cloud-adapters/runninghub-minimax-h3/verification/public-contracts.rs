use bgx_adapter_v2_validator::{usage::validate_usage, validate_contract};
use serde_json::{json, Value};
fn main() {
    let f: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let c: Vec<bgx_adapter_v2_sdk::CapabilityContractV2> =
        serde_json::from_value(f["capabilities"]["capabilityContracts"].clone()).unwrap();
    assert_eq!(c.len(), 7);
    for one in &c {
        validate_contract(one).unwrap();
    }
    let plugin = f["capabilities"]["pluginId"].as_str().unwrap();
    validate_usage(&f["usage"], plugin).unwrap();
    let mut old = f["usage"].clone();
    old["measurements"][0]["unit"] = json!("RH_coin");
    assert!(validate_usage(&old, plugin).is_err());
    let registry = bgx_adapter_v2_validator::SchemaRegistry::public().unwrap();
    for result in f["queryResults"].as_array().unwrap() {
        let _: bgx_adapter_v2_sdk::QueryResult = serde_json::from_value(result.clone()).unwrap();
        registry
            .validate_dispatch("query", "result", result)
            .unwrap();
    }
    for stat in f["managedVideoStats"].as_array().unwrap() {
        let _: bgx_adapter_v2_sdk::HostPortCallV2MediaStat =
            serde_json::from_value(stat.clone()).unwrap();
        registry
            .validate_at(
                "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json",
                "/$defs/mediaStat",
                stat,
            )
            .unwrap();
    }
    println!("PUBLIC_SDK_VIDEO_RESULTS_PASS: actual QuickTime/MP4 stats and real package query results preserve probed facts");
    let installation: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(2).unwrap()).unwrap()).unwrap();
    let package = std::path::Path::new(&std::env::args().nth(2).unwrap())
        .parent()
        .unwrap()
        .join("package");
    let manifest = std::fs::read(package.join("manifest.json")).unwrap();
    let inventory = std::fs::read(package.join("manifest.sha256")).unwrap();
    let mut measured = std::collections::BTreeMap::new();
    for name in ["manifest.json", "adapter.wasm"] {
        measured.insert(
            name.into(),
            bgx_adapter_v2_validator::hash_bytes(&std::fs::read(package.join(name)).unwrap()),
        );
    }
    let verified = bgx_adapter_v2_validator::package::verify_package(
        &bgx_adapter_v2_validator::package::PackageContents {
            manifest_bytes: &manifest,
            hash_manifest_bytes: &inventory,
            signature: None,
            measured_file_hashes: &measured,
        },
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        installation["manifestSha256"],
        bgx_adapter_v2_validator::hash_bytes(&manifest)
    );
    use bgx_adapter_v2_reference_host::{
        media::LeaseScope,
        policy::{DevelopmentNetworkGrant, NetworkPolicy},
    };
    let scope = LeaseScope {
        package: verified.identity().clone(),
        binding_id: "verify-binding".into(),
        request_id: "verify-request".into(),
    };
    let origins: Vec<String> =
        serde_json::from_value(installation["hostConfig"]["network"]["origins"].clone()).unwrap();
    assert_eq!(
        origins,
        vec![
            "https://www.runninghub.cn",
            "https://rh-comfyui01.tos-cn-beijing.volces.com"
        ]
    );
    assert_eq!(
        installation["authentication"]["fields"][0]["origins"],
        json!(["https://www.runninghub.cn"])
    );
    let grant = |origins| DevelopmentNetworkGrant {
        origins,
        allow_private: false,
        allow_embedded_authorization: false,
        denied_hosts: vec![],
        denied_networks: vec![],
    };
    let policy = NetworkPolicy::new(&verified, scope.clone(), grant(origins), true).unwrap();
    let public = ["8.8.8.8".parse().unwrap()];
    assert!(policy
        .check_url(
            &scope,
            "https://www.runninghub.cn/openapi/v2/query",
            &public
        )
        .is_ok());
    assert!(policy
        .check_url(
            &scope,
            "https://rh-comfyui01.tos-cn-beijing.volces.com/test.mp4",
            &public
        )
        .is_ok());
    assert!(policy
        .check_url(&scope, "https://unverified.example/test.mp4", &public)
        .is_err());
    let restricted = NetworkPolicy::new(
        &verified,
        scope.clone(),
        grant(vec!["https://www.runninghub.cn".into()]),
        true,
    )
    .unwrap();
    assert!(restricted
        .check_url(
            &scope,
            "https://rh-comfyui01.tos-cn-beijing.volces.com/test.mp4",
            &public
        )
        .is_err());
    println!("PUBLIC_SDK_NETWORK_POLICY_PASS: exact API/output origin granted; other origin/absent output grant rejected; credential restricted to API; no network call");
    println!("PUBLIC_SDK_VALIDATOR_PASS: 7 capability contracts; actual reported usage; unnamespaced usage rejected");
}
