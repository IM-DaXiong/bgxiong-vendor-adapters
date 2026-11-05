use bgx_adapter_v2_ports::resource_policy;
use bgx_adapter_v2_reference_host::media::*;
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::identity::PackageIdentity;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    time::Duration,
};

fn scope() -> LeaseScope {
    LeaseScope {
        package: PackageIdentity {
            plugin_id: "local.media".into(),
            version: "1.0.0".into(),
            digest: "0".repeat(64),
        },
        binding_id: "binding".into(),
        request_id: "request".into(),
    }
}
fn setup() -> (tempfile::TempDir, DevelopmentWorkspace, MediaBroker) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let broker = MediaBroker::new(workspace.clone(), Duration::from_secs(60), None).unwrap();
    (directory, workspace, broker)
}
fn image(path: &Path, format: image::ImageFormat) {
    let pixels = image::RgbImage::from_pixel(3, 2, image::Rgb([40, 50, 60]));
    pixels.save_with_format(path, format).unwrap();
}

#[test]
fn content_identity_survives_restart_but_handles_and_scope_do_not() {
    let (directory, workspace, mut broker) = setup();
    let source = directory.path().join("wrong-extension.png");
    image(&source, image::ImageFormat::Jpeg);
    let owner = scope();
    let control = Control::new(None);
    let lease = broker
        .import_file(&owner, "source", &source, "image", &control)
        .unwrap();
    assert_eq!(lease.stat.r#mime, "image/jpeg");
    assert_eq!(lease.stat.r#width, Some(3));
    assert_eq!(lease.stat.r#height, Some(2));
    assert!(lease.file_name.ends_with(".jpg"));
    let actual = fs::read(&source).unwrap();
    assert_eq!(
        broker
            .read(&owner, &lease.handle, 0, actual.len(), &control)
            .unwrap(),
        actual
    );
    let mut other = owner.clone();
    other.request_id = "other".into();
    assert!(broker.info(&other, &lease.handle).is_err());
    other = owner.clone();
    other.binding_id = "foreign".into();
    assert!(broker.info(&other, &lease.handle).is_err());
    assert!(broker.rehydrate(&other, &lease.identity, &control).is_err());
    other = owner.clone();
    other.package.digest = "f".repeat(64);
    assert!(broker.rehydrate(&other, &lease.identity, &control).is_err());
    let context = broker.validation_context(&owner);
    assert_eq!(
        context.leases_by_handle[&lease.handle].facts.file_name,
        lease.file_name
    );
    assert_eq!(
        context.leases_by_handle[&lease.handle].facts.byte_length,
        actual.len() as u64
    );
    let descriptor: bgx_adapter_v2_ports::wire::MediaInputV2 = serde_json::from_value(serde_json::json!({
        "id": lease.identity.r#id, "kind":"image", "handle":lease.handle,
        "mime":lease.stat.r#mime, "fileName":lease.file_name, "byteLength":lease.stat.r#byte_length
    })).unwrap();
    broker.close().unwrap();
    assert_eq!(fs::read(&source).unwrap(), actual);
    let reopened = DevelopmentWorkspace::open(workspace.root()).unwrap();
    let mut broker = MediaBroker::new(reopened, Duration::from_secs(60), None).unwrap();
    assert!(broker.info(&owner, &lease.handle).is_err());
    let mut resumed = owner.clone();
    resumed.request_id = "resume-process".into();
    let restored = broker.rehydrate_descriptor(&resumed, &descriptor, &control).unwrap();
    assert_ne!(restored.handle, descriptor.r#handle);
    assert_eq!(broker.read(&resumed, &restored.handle, 0, actual.len(), &control).unwrap(), actual);
    broker.release(&resumed, &restored.handle).unwrap();
    let mut mismatched = descriptor.clone();
    mismatched.r#byte_length = "1".into();
    assert!(broker.rehydrate_descriptor(&resumed, &mismatched, &control).is_err());
    let mut foreign = resumed.clone();
    foreign.binding_id = "foreign".into();
    assert!(broker.rehydrate_descriptor(&foreign, &descriptor, &control).is_err());

    let new = broker
        .rehydrate(&resumed, &lease.identity, &control)
        .unwrap();
    assert_ne!(new.handle, lease.handle);
    assert_eq!(
        broker
            .read(&resumed, &new.handle, 0, actual.len(), &control)
            .unwrap(),
        actual
    );
    broker.release(&resumed, &new.handle).unwrap();
    assert!(broker.info(&resumed, &new.handle).is_err());
    assert!(broker
        .rehydrate(&resumed, &lease.identity, &control)
        .is_ok());
    broker.close().unwrap();
}

#[test]
fn large_files_are_copied_and_read_in_bounded_chunks_and_tampering_is_rejected() {
    let (directory, _, mut broker) = setup();
    let source = directory.path().join("large.binary");
    let mut file = File::create(&source).unwrap();
    let chunk = vec![0x58u8; 65536];
    for _ in 0..512 {
        file.write_all(&chunk).unwrap();
    }
    drop(file);
    let owner = scope();
    let control = Control::new(None);
    let lease = broker
        .import_file(&owner, "large", &source, "file", &control)
        .unwrap();
    assert_eq!(lease.stat.r#byte_length, "33554432");
    let limit = resource_policy()["limits"]["maxMediaChunkBytes"]
        .as_u64()
        .unwrap() as usize;
    let mut offset = 0u64;
    loop {
        let bytes = broker
            .read(&owner, &lease.handle, offset, limit, &control)
            .unwrap();
        assert!(bytes.len() <= limit);
        assert!(bytes.iter().all(|b| *b == 0x58));
        if bytes.is_empty() {
            break;
        }
        offset += bytes.len() as u64;
    }
    assert_eq!(offset, 33554432);
    assert!(broker
        .read(&owner, &lease.handle, 0, limit + 1, &control)
        .is_err());
    assert!(broker
        .read(&owner, &lease.handle, offset + 1, 1, &control)
        .is_err());
    let object = directory.path().join("objects").join(&lease.file_name);
    File::options()
        .write(true)
        .open(&object)
        .unwrap()
        .write_all(b"tampered")
        .unwrap();
    assert!(broker.rehydrate(&owner, &lease.identity, &control).is_err());
    assert!(broker.read(&owner, &lease.handle, 0, 10, &control).is_err());
    broker.close().unwrap();
    assert!(source.exists());
}

#[test]
fn staged_output_finalizes_real_extension_and_failure_release_cleans_partial_files() {
    let (directory, _, mut broker) = setup();
    let owner = scope();
    let control = Control::new(None);
    let source = directory.path().join("source.png");
    image(&source, image::ImageFormat::Png);
    let bytes = fs::read(source).unwrap();
    let handle = broker.create(&owner, "image", "image/png").unwrap();
    assert!(broker.info(&owner, &handle).is_err());
    for chunk in bytes.chunks(11) {
        broker.write(&owner, &handle, chunk, &control).unwrap();
    }
    let done = broker.finish(&owner, &handle, &control).unwrap();
    assert!(done.file_name.ends_with(".png"));
    assert_eq!(done.stat.r#mime, "image/png");
    assert!(broker.write(&owner, &handle, b"bad", &control).is_err());
    assert!(broker.finish(&owner, &handle, &control).is_err());
    let partial = broker.create(&owner, "image", "image/png").unwrap();
    broker
        .write(&owner, &partial, b"not an image", &control)
        .unwrap();
    assert!(broker.finish(&owner, &partial, &control).is_err());
    assert!(broker.info(&owner, &partial).is_err());
    let cancelled = broker
        .create(&owner, "file", "application/octet-stream")
        .unwrap();
    broker
        .write(&owner, &cancelled, b"partial", &control)
        .unwrap();
    control.cancel();
    assert!(broker.write(&owner, &cancelled, b"more", &control).is_err());
    broker.release_request(&owner).unwrap();
    assert!(broker.info(&owner, &done.handle).is_err());
    assert_eq!(
        fs::read_dir(directory.path().join("staging"))
            .unwrap()
            .flat_map(|session| fs::read_dir(session.unwrap().path()).unwrap())
            .count(),
        0
    );
    // Final content is durable despite releasing the request's leases.
    assert!(directory
        .path()
        .join("objects")
        .join(done.file_name)
        .exists());
    broker.close().unwrap();
    assert_eq!(
        fs::read_dir(directory.path().join("staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn empty_populated_foreign_expired_and_invalid_probe_workspaces_fail_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("existing"), b"untouched").unwrap();
    assert!(DevelopmentWorkspace::create(directory.path()).is_err());
    assert!(DevelopmentWorkspace::open(directory.path()).is_err());
    assert_eq!(
        fs::read(directory.path().join("existing")).unwrap(),
        b"untouched"
    );
    let (workspace_root, workspace, broker) = setup();
    broker.close().unwrap();
    let mut broker = MediaBroker::new(workspace.clone(), Duration::from_millis(20), None).unwrap();
    let owner = scope();
    let handle = broker
        .create(&owner, "file", "application/octet-stream")
        .unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(broker.info(&owner, &handle).is_err());
    broker.reap_expired().unwrap();
    assert!(broker.release(&owner, &handle).is_err());
    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(broker
        .import_file(
            &owner,
            "outside",
            outside.path(),
            "file",
            &Control::new(None)
        )
        .is_err());
    assert!(MediaBroker::new(
        workspace,
        Duration::from_secs(60),
        Some(ProbeTool {
            executable: workspace_root.path().join("missing-ffprobe.exe"),
            timeout: Duration::from_secs(1),
            frame_timestamps: false
        })
    )
    .is_err());
    broker.close().unwrap();
}

#[test]
fn explicit_probe_protocol_reports_actual_timestamps_and_reaps_faulting_tools() {
    let (directory, workspace, broker) = setup();
    broker.close().unwrap();
    // This executable is an explicitly labelled probe protocol fixture, not an
    // ffprobe measurement certification. Real ffprobe acceptance remains later.
    let source = r#"use std::{io::{self,Write},fs,path::PathBuf,thread,time::Duration};fn main(){let args:Vec<_>=std::env::args_os().collect();let path=PathBuf::from(args.last().unwrap());let mode=fs::read_to_string(path.parent().unwrap().join("probe-mode")).unwrap();match mode.as_str(){"hang"=>thread::sleep(Duration::from_secs(10)),"overflow"=>{let _=io::stdout().write_all(&vec![b'x';9000000]);},"malformed"=>print!("{{"),_=>print!("{{\"streams\":[{{\"codec_type\":\"video\",\"width\":16,\"height\":8,\"duration\":\"0.200\"}}],\"frames\":[{{\"media_type\":\"video\",\"best_effort_timestamp_time\":\"-1.000\"}},{{\"media_type\":\"video\",\"best_effort_timestamp_time\":\"-0.960\"}},{{\"media_type\":\"video\",\"best_effort_timestamp_time\":\"-0.880\"}}]}}")}}"#;
    let tool = directory.path().join(if cfg!(windows) {
        "probe-fixture.exe"
    } else {
        "probe-fixture"
    });
    fs::write(directory.path().join("probe.rs"), source).unwrap();
    let built = std::process::Command::new("rustc")
        .args(["--edition=2021"])
        .arg(directory.path().join("probe.rs"))
        .arg("-o")
        .arg(&tool)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let video = directory.path().join("input.mp4");
    fs::write(&video, b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom").unwrap();
    let mut broker = MediaBroker::new(
        workspace,
        Duration::from_secs(60),
        Some(ProbeTool {
            executable: tool,
            timeout: Duration::from_millis(200),
            frame_timestamps: true,
        }),
    )
    .unwrap();
    // Probe runs against its own staging snapshot. Test fixture mode is explicit
    // in that session directory, which is only a development test workspace.
    let session = fs::read_dir(directory.path().join("staging"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let control = Control::new(None);
    fs::write(session.join("probe-mode"), "good").unwrap();
    let lease = broker
        .import_file(&scope(), "vfr", &video, "video", &control)
        .unwrap();
    assert_eq!(lease.stat.r#duration_ms, Some("200".into()));
    assert_eq!(
        lease.stat.r#timestamps_ms,
        Some(vec!["0".into(), "40".into(), "120".into()])
    );
    for mode in ["hang", "overflow", "malformed"] {
        fs::write(session.join("probe-mode"), mode).unwrap();
        assert!(broker
            .import_file(&scope(), mode, &video, "video", &control)
            .is_err());
    }
    fs::remove_file(session.join("probe-mode")).unwrap();
    broker.close().unwrap();
}

/// This acceptance test never substitutes a synthetic probe. The caller must
/// explicitly provide a real generated VFR fixture and a real ffprobe binary.
#[test]
#[ignore = "requires BGX_REAL_FFPROBE, BGX_REAL_VFR and new BGX_REAL_MEDIA_PROOF absolute paths"]
fn real_vfr_probe_drives_pixel_mask_and_millisecond_validation() {
    use bgx_adapter_v2_sdk::{CapabilityContractV2, OperationSubmitV2};
    use bgx_adapter_v2_validator::{hash_bytes, validate_input_with_media};
    use serde_json::{json, Value};
    use std::path::PathBuf;
    let explicit = |key: &str| {
        let path = PathBuf::from(std::env::var_os(key).expect(key));
        assert!(path.is_absolute(), "explicit absolute path required: {key}");
        path
    };
    let tool = explicit("BGX_REAL_FFPROBE");
    let source = explicit("BGX_REAL_VFR");
    let proof = explicit("BGX_REAL_MEDIA_PROOF");
    assert!(!proof.exists(), "proof must be new");
    let (directory, workspace, broker) = setup();
    broker.close().unwrap();
    let mut broker = MediaBroker::new(
        workspace.clone(),
        Duration::from_secs(60),
        Some(ProbeTool {
            executable: tool.clone(),
            timeout: Duration::from_secs(15),
            frame_timestamps: true,
        }),
    )
    .unwrap();
    let owner = scope();
    let control = Control::new(None);
    let granted_source = directory.path().join("explicit-vfr-source.mp4");
    fs::copy(&source, &granted_source).unwrap();
    let video = broker
        .import_file(&owner, "source", &granted_source, "video", &control)
        .unwrap();
    let width = video.stat.width.unwrap();
    let height = video.stat.height.unwrap();
    let duration: u64 = video.stat.duration_ms.as_ref().unwrap().parse().unwrap();
    let timestamps: Vec<u64> = video
        .stat
        .timestamps_ms
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert!(timestamps.len() >= 3 && timestamps[0] == 0);
    let deltas: Vec<_> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(deltas.iter().all(|d| *d > 0));
    assert!(
        deltas.windows(2).any(|d| d[0] != d[1]),
        "fixture must really be VFR"
    );
    assert!(width > 1 && height > 1 && duration >= *timestamps.last().unwrap());
    let mask_source = directory.path().join("mask.png");
    image::GrayImage::from_pixel(
        width.try_into().unwrap(),
        height.try_into().unwrap(),
        image::Luma([255]),
    )
    .save_with_format(&mask_source, image::ImageFormat::Png)
    .unwrap();
    let mask = broker
        .import_file(&owner, "mask", &mask_source, "image", &control)
        .unwrap();
    let wrong_mask_source = directory.path().join("wrong-mask.png");
    image::GrayImage::from_pixel(
        (width + 1).try_into().unwrap(),
        height.try_into().unwrap(),
        image::Luma([255]),
    )
    .save_with_format(&wrong_mask_source, image::ImageFormat::Png)
    .unwrap();
    let wrong_mask = broker
        .import_file(&owner, "wrong-mask", &wrong_mask_source, "image", &control)
        .unwrap();
    let media = [&video, &mask, &wrong_mask].map(|m| json!({
        "id": if m.handle == video.handle {"source"} else if m.handle == mask.handle {"mask"} else {"wrong-mask"},
        "kind": m.stat.kind, "handle": m.handle, "mime": m.stat.mime,
        "fileName": m.file_name, "byteLength": m.stat.byte_length
    }));
    let context = broker.validation_context(&owner);
    let base: Value =
        serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut checks = Vec::new();
    for (widget, good, bad) in [
        (
            "point",
            json!({"mediaId":"source","x":width as f64-0.5,"y":height-1}),
            json!({"mediaId":"source","x":width,"y":height-1}),
        ),
        (
            "box",
            json!({"mediaId":"source","x":0,"y":0,"width":width,"height":height}),
            json!({"mediaId":"source","x":0,"y":0,"width":width+1,"height":height}),
        ),
        (
            "time_span",
            json!({"mediaId":"source","startMs":timestamps[1],"endMs":duration}),
            json!({"mediaId":"source","startMs":timestamps[1],"endMs":duration+1}),
        ),
        (
            "mask",
            json!({"mediaId":"source","maskMediaId":"mask"}),
            json!({"mediaId":"source","maskMediaId":"wrong-mask"}),
        ),
    ] {
        let mut declaration = base["capabilities"][0].clone();
        let schema = json!({"type":"object","properties":{
            "source":{"type":"object","properties":{"mediaId":{"type":"string"}},"required":["mediaId"],"additionalProperties":false},
            "selection":{"$ref":format!("https://bgxiong.local/vendor-adapter/v2/interaction-values.schema.json#/$defs/{widget}")}
        },"required":["source","selection"],"additionalProperties":false}).to_string();
        let digest = hash_bytes(schema.as_bytes());
        for resource in declaration["schemas"].as_array_mut().unwrap() {
            if resource["id"] == "input" {
                resource["schemaJson"] = json!(schema);
                resource["digest"] = json!(digest);
            }
        }
        declaration["operations"][0]["inputSchema"]["digest"] = json!(digest);
        declaration["operations"][0]["uiSchema"]["controls"] = json!([
            {"fieldPointer":"/source","widget":"media","label":"Source","description":"","group":""},
            {"fieldPointer":"/selection","widget":widget,"label":"Selection","description":"","group":""}
        ]);
        declaration["operations"][0]["mediaSlots"] = json!([{"role":"source","inputPointer":"/source","mediaKind":"video","minCount":1,"maxCount":null}]);
        let contract: CapabilityContractV2 = serde_json::from_value(declaration).unwrap();
        let make = |selection| -> OperationSubmitV2 {
            serde_json::from_value(json!({
            "schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,
            "inputSchemaDigest":digest,"input":{"source":{"mediaId":"source"},"selection":selection},
            "media":media,"uploads":[],"configuration":{"endpointBaseUrl":"https://example.invalid"}
        })).unwrap()
        };
        validate_input_with_media(&contract, &make(good.clone()), &context).unwrap();
        assert!(validate_input_with_media(&contract, &make(bad.clone()), &context).is_err());
        let mut foreign = good.clone();
        foreign["mediaId"] = json!("foreign");
        assert!(validate_input_with_media(&contract, &make(foreign), &context).is_err());
        checks.push(
            json!({"widget":widget,"accepted":good,"rejected":bad,"foreignMediaRejected":true}),
        );
    }
    let identity = video.identity.clone();
    broker.close().unwrap();
    let mut reopened = MediaBroker::new(
        DevelopmentWorkspace::open(workspace.root()).unwrap(),
        Duration::from_secs(60),
        Some(ProbeTool {
            executable: tool.clone(),
            timeout: Duration::from_secs(15),
            frame_timestamps: true,
        }),
    )
    .unwrap();
    assert!(reopened.info(&owner, &video.handle).is_err());
    let mut resumed = owner.clone();
    resumed.request_id = "real-vfr-resume".into();
    let restored = reopened.rehydrate(&resumed, &identity, &control).unwrap();
    assert_ne!(restored.handle, video.handle);
    assert_eq!(restored.stat.timestamps_ms, video.stat.timestamps_ms);
    reopened.release(&resumed, &restored.handle).unwrap();
    assert!(reopened.info(&resumed, &restored.handle).is_err());
    reopened.close().unwrap();
    let output = json!({"schemaVersion":1,"kind":"REAL_PROBE_GEOMETRY_EXECUTION",
        "probe":{"path":tool,"sha256":hash_bytes(&fs::read(&tool).unwrap())},
        "fixture":{"path":source,"sha256":hash_bytes(&fs::read(&source).unwrap())},
        "measured":{"width":width,"height":height,"durationMs":duration,"timestampsMs":timestamps},
        "checks":checks,"restart":{"oldHandleRejected":true,"timestampsPreserved":true,"newHandleDistinct":true,"releasedHandleRejected":true}});
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(proof)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&output).unwrap())
        .unwrap();
    file.sync_all().unwrap();
}
