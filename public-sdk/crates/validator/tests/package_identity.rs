use std::collections::{BTreeMap, BTreeSet};
use bgx_adapter_v2_validator::{hash_bytes, package::*, identity::*};
use serde_json::{json, Value};

fn manifest() -> Value {
    json!({"schemaVersion":2,"pluginId":"local.author","displayName":"Test","version":"1.0.0",
        "protocolRange":{"min":2,"max":2},"requiredFeatures":[],"optionalFeatures":[],"language":"rust",
        "entry":"adapter.wasm","artifactSha256":hash_bytes(b"wasm-test-only"),"publisherId":null,"publisherKeyId":null,
        "requestedCapabilities":{"endpointPolicy":["userConfigurable"],"networkHosts":[],"allowPrivateNetwork":false,
            "auth":["none"],"mediaSlots":[],"stateBytes":0,"embeddedHeaders":[]},
        "limits":{"memoryBytes":65536,"fuel":1000,"wallMs":100,"outputBytes":256}})
}
fn pack(m: &Value) -> (Vec<u8>, Vec<u8>, BTreeMap<String,String>) {
    let bytes = serde_json::to_vec(m).unwrap();
    let files = BTreeMap::from([("manifest.json".into(),hash_bytes(&bytes)),("adapter.wasm".into(),hash_bytes(b"wasm-test-only"))]);
    let hashes = files.iter().map(|(file,hash)| format!("{hash} {file}\n")).collect::<String>().into_bytes();
    (bytes,hashes,files)
}
fn check(m: &Value, trust: &TrustContext) -> bool {
    let (bytes,hashes,files) = pack(m);
    verify_package(&PackageContents{manifest_bytes:&bytes,hash_manifest_bytes:&hashes,signature:None,measured_file_hashes:&files},trust).is_ok()
}

#[test]
fn unsigned_identity_integrity_and_feature_negotiation() {
    let m=manifest(); let trust=TrustContext::default(); assert!(check(&m,&trust));
    let (bytes,hashes,mut files)=pack(&m);
    let verified=verify_package(&PackageContents{manifest_bytes:&bytes,hash_manifest_bytes:&hashes,signature:None,measured_file_hashes:&files},&trust).unwrap();
    assert!(!verified.is_trusted()); assert_eq!(verified.namespace(),"local.author");
    files.insert("adapter.wasm".into(),hash_bytes(b"tampered"));
    assert!(verify_package(&PackageContents{manifest_bytes:&bytes,hash_manifest_bytes:&hashes,signature:None,measured_file_hashes:&files},&trust).is_err());
    for mutation in ["protocol","publisher","feature","path","artifact","version"] {
        let mut bad=m.clone(); match mutation {
            "protocol"=>bad["protocolRange"]["min"]=json!(1),
            "publisher"=>bad["pluginId"]=json!("official.fake"),
            "feature"=>bad["requiredFeatures"]=json!(["unimplemented-feature"]),
            "path"=>bad["entry"]=json!("../escape.wasm"),
            "artifact"=>bad["artifactSha256"]=json!("0".repeat(64)),
            "version"=>bad["version"]=json!("not-semver"), _=>unreachable!() }
        assert!(!check(&bad,&trust),"{mutation}");
    }
    let mut feature=m;feature["requiredFeatures"]=json!(["stream-events-v1"]);
    let trust=TrustContext{supported_features:BTreeSet::from(["stream-events-v1".into()]),..Default::default()};
    assert!(check(&feature,&trust));
    assert!(validate_operation_namespace("local.author","local.author/new-operation").is_ok());
    assert!(validate_operation_namespace("local.author","somebody-else/operation").is_err());
    assert!(validate_operation_namespace("local.author","std/not-published").is_err());
    assert!(validate_capability_namespace("local.author","video.edit").is_ok());
}

#[test]
fn real_signature_is_verified_and_cannot_be_replaced_with_a_trusted_flag() {
    use base64::{Engine,engine::general_purpose::STANDARD};
    use blake2::{Blake2b512,Digest};
    use ed25519_dalek::{Signer,SigningKey};
    let key=SigningKey::from_bytes(&[9u8;32]);let key_id=[1u8;8];
    let mut m=manifest();m["pluginId"]=json!("author.adapter");m["publisherId"]=json!("author");m["publisherKeyId"]=json!("dev-key");
    let (bytes,hashes,files)=pack(&m);
    let signature=key.sign(&Blake2b512::digest(&hashes)).to_bytes();
    let trusted="test-only-prehashed";
    let global=key.sign(&[signature.as_slice(),trusted.as_bytes()].concat()).to_bytes();
    let first=[b"ED".as_slice(),key_id.as_slice(),signature.as_slice()].concat();
    let sig=format!("untrusted comment: test\n{}\ntrusted comment: {trusted}\n{}\n",STANDARD.encode(first),STANDARD.encode(global));
    let pubkey=STANDARD.encode([b"Ed".as_slice(),key_id.as_slice(),key.verifying_key().as_bytes().as_slice()].concat());
    let public_key=format!("untrusted comment: test\n{pubkey}\n");
    let trust=TrustContext{is_fresh:true,publishers:vec![PublisherTrust{publisher_id:"author".into(),key_id:"dev-key".into(),
        public_key,plugin_prefixes:vec!["author.".into()]}],..Default::default()};
    let content=PackageContents{manifest_bytes:&bytes,hash_manifest_bytes:&hashes,signature:Some(&sig),measured_file_hashes:&files};
    assert!(verify_package(&content,&trust).unwrap().is_trusted());
    let bad=PackageContents{signature:Some("not-a-signature"),..content};
    assert!(verify_package(&bad,&trust).is_err());
    let trust=TrustContext{revoked_keys:BTreeSet::from(["dev-key".into()]),..trust};
    assert!(verify_package(&content,&trust).is_err());
}
