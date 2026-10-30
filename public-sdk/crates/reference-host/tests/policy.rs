use bgx_adapter_v2_ports::wire::HostPortCallV2HttpRequest;
use bgx_adapter_v2_reference_host::{credential::*, http::*, media::*, policy::*};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{hash_bytes, package::*};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

fn package(embedded: bool) -> VerifiedPackage {
    let manifest = json!({"schemaVersion":2,"pluginId":"local.policy","displayName":"Policy tests","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":[],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(b"fixture"),"publisherId":null,"publisherKeyId":null,
        "requestedCapabilities":{"endpointPolicy":["userConfigurable","allowPrivateNetwork"],"networkHosts":[],"allowPrivateNetwork":true,"auth":["none","bearer","custom-header","jwt-hs256","hmac-sha256","hmac-sha256-credential"],"mediaSlots":["file","image"],"stateBytes":0,"embeddedHeaders":if embedded{vec!["authorization"]}else{vec![]}},"limits":{"memoryBytes":33554432,"fuel":1000,"wallMs":100,"outputBytes":1048576}});
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let hashes = BTreeMap::from([
        ("manifest.json".into(), hash_bytes(&bytes)),
        ("adapter.wasm".into(), hash_bytes(b"fixture")),
    ]);
    let inventory = hashes
        .iter()
        .map(|(k, v)| format!("{v} {k}\n"))
        .collect::<String>();
    verify_package(
        &PackageContents {
            manifest_bytes: &bytes,
            hash_manifest_bytes: inventory.as_bytes(),
            signature: None,
            measured_file_hashes: &hashes,
        },
        &TrustContext::default(),
    )
    .unwrap()
}
fn scope(p: &VerifiedPackage, id: &str) -> LeaseScope {
    LeaseScope {
        package: p.identity().clone(),
        binding_id: "test".into(),
        request_id: id.into(),
    }
}
fn policy(p: &VerifiedPackage, id: &str, origins: &[&str], private: bool) -> NetworkPolicy {
    NetworkPolicy::new(
        p,
        scope(p, id),
        DevelopmentNetworkGrant {
            origins: origins.iter().map(|s| s.to_string()).collect(),
            allow_private: private,
            allow_embedded_authorization: false,
            denied_hosts: vec![],
            denied_networks: vec![],
        },
        true,
    )
    .unwrap()
}
fn plan(url: &str) -> Value {
    json!({"method":"GET","url":url,"headers":[],"generatedHeaders":[],"body":{"kind":"none"},"sink":{"kind":"buffer","maxBytes":10000},"timeoutMs":2000})
}
fn material(secret: &[u8]) -> DevelopmentSecret {
    DevelopmentSecret {
        bytes: secret.to_vec().into(),
        public_id: Some("public-id".into()),
        custom_header: Some("X-Token".into()),
    }
}

#[test]
fn network_gate_and_credentials_have_no_implicit_privilege_or_secret_return() {
    let package = package(false);
    let p = policy(&package, "request", &["https://api.example.invalid"], false);
    let scope = p.scope();
    assert!(p
        .check_url(
            scope,
            "https://api.example.invalid/x",
            &["8.8.8.8".parse().unwrap()]
        )
        .is_ok());
    for raw in [
        "https://api.example.invalid.evil/x",
        "https://user:pass@api.example.invalid/x",
        "file:///tmp/data",
        "https://api.example.invalid/x#fragment",
    ] {
        assert!(p
            .check_url(scope, raw, &["8.8.8.8".parse().unwrap()])
            .is_err());
    }
    for ip in [
        "127.0.0.1",
        "169.254.169.254",
        "0.0.0.0",
        "224.0.0.1",
        "::",
        "fe80::1",
        "::ffff:169.254.169.254",
    ] {
        assert!(
            p.check_url(
                scope,
                "https://api.example.invalid/x",
                &[ip.parse().unwrap()]
            )
            .is_err(),
            "ip={ip}"
        );
    }
    // One forbidden DNS answer poisons the destination; never select only the
    // convenient public address from a rebinding/mixed result set.
    assert!(p
        .check_url(
            scope,
            "https://api.example.invalid",
            &["8.8.8.8".parse().unwrap(), "127.0.0.1".parse().unwrap()]
        )
        .is_err());
    let local = policy(&package, "request", &["http://127.0.0.1:1234"], true);
    assert!(local
        .check_url(
            local.scope(),
            "http://127.0.0.1:1234/x",
            &["127.0.0.1".parse().unwrap()]
        )
        .is_ok());
    let public_http = policy(&package, "request", &["http://8.8.8.8"], true);
    assert!(public_http
        .check_url(
            public_http.scope(),
            "http://8.8.8.8",
            &["8.8.8.8".parse().unwrap()]
        )
        .is_err());
    let mut vault = CredentialVault::new().unwrap();
    let origins = p.origins().to_vec();
    let key = vault
        .insert(
            &p,
            material(&[0x0b; 20]),
            BTreeSet::from(["hmac-sha256".into()]),
            origins.clone(),
            Duration::from_secs(60),
        )
        .unwrap();
    assert_eq!(
        vault
            .sign(
                &p,
                &key,
                &json!({"version":1,"kind":"hmac-sha256","message":"Hi There","encoding":"hex"})
                    .to_string()
            )
            .unwrap(),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    assert!(vault.sign(&p,&key,&json!({"version":1,"kind":"hmac-sha256","message":"Hi There","encoding":"hex","unknown":true}).to_string()).is_err());
    let secret = "explicit-development-key";
    let bearer = vault
        .insert(
            &p,
            material(secret.as_bytes()),
            BTreeSet::from([
                "bearer".into(),
                "jwt-hs256".into(),
                "hmac-sha256-credential".into(),
            ]),
            origins,
            Duration::from_secs(60),
        )
        .unwrap();
    assert!(vault
        .sign(
            &p,
            &bearer,
            &json!({"version":1,"kind":"bearer"}).to_string()
        )
        .is_err());
    assert!(vault.capabilities(&p, "forged").is_err());
    let other = policy(
        &package,
        "other-request",
        &["https://api.example.invalid"],
        false,
    );
    assert!(vault.capabilities(&other, &bearer).is_err());
    let token = vault
        .sign(
            &p,
            &bearer,
            &json!({"version":1,"kind":"jwt-hs256","claimsJson":"{\"sub\":\"example\"}"})
                .to_string(),
        )
        .unwrap();
    let parts: Vec<_> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(header, json!({"alg":"HS256","typ":"JWT"}));
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(claims, json!({"sub":"example"}));
    use hmac::{Hmac, Mac};
    let mut expected = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    expected.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
    expected
        .verify_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap())
        .unwrap();
    let mut presented = material(secret.as_bytes());
    presented.public_id = Some("id-{date}".into());
    let presented = vault
        .insert(
            &p,
            presented,
            BTreeSet::from(["hmac-sha256-credential".into()]),
            p.origins().to_vec(),
            Duration::from_secs(60),
        )
        .unwrap();
    let signature=vault.sign(&p,&presented,&json!({"version":1,"kind":"hmac-sha256-credential","canonical":"canonical request","date":"20261019T000000Z"}).to_string()).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(b"canonical request");
    assert_eq!(
        signature,
        format!(
            "HMAC-SHA256 Credential=id-{{date}}, SignedDate=20261019T000000Z, Signature={}",
            hex::encode(mac.finalize().into_bytes())
        )
    );
    let custom = vault
        .insert(
            &p,
            material(b"custom-development-key"),
            BTreeSet::from(["custom-header".into()]),
            p.origins().to_vec(),
            Duration::from_secs(60),
        )
        .unwrap();
    let mut custom_plan = plan("https://api.example.invalid");
    custom_plan["generatedHeaders"] =
        json!([{"name":"X-Token","kind":"custom-header","targetField":"x-token"}]);
    assert_eq!(
        vault
            .headers(&p, &p.origins()[0], &custom_plan, Some(&custom))
            .unwrap()["x-token"],
        "custom-development-key"
    );
    assert!(vault
        .insert(
            &p,
            material(b"wrong-mode"),
            BTreeSet::from(["http-sigv4".into()]),
            p.origins().to_vec(),
            Duration::from_secs(60)
        )
        .is_err());
    let mut request = plan("https://api.example.invalid/x");
    for name in [
        "Host",
        "Content-Length",
        "Cookie",
        "Authorization",
        "Transfer-Encoding",
    ] {
        request["headers"] = json!([{"name":name,"value":"forged"}]);
        assert!(vault.headers(&p, &p.origins()[0], &request, None).is_err());
    }
    request["headers"] = json!([{"name":"Authorization","value":format!("Bearer {token}")}]);
    assert!(vault.headers(&p, &p.origins()[0], &request, None).is_ok());
    assert!(vault
        .headers(&other, &p.origins()[0], &request, None)
        .is_err());
    request["headers"] = json!([]);
    request["generatedHeaders"] =
        json!([{"name":"Authorization","kind":"bearer","targetField":"authorization"}]);
    let headers = vault
        .headers(&p, &p.origins()[0], &request, Some(&bearer))
        .unwrap();
    assert_eq!(headers["authorization"], format!("Bearer {secret}"));
    assert!(headers["authorization"].is_sensitive());
    request["generatedHeaders"][0]["targetField"] = json!("/arbitrary-body-field");
    assert!(vault
        .headers(&p, &p.origins()[0], &request, Some(&bearer))
        .is_err());
    assert!(!vault
        .redact(&format!("failed with {secret}"))
        .contains(secret));
    vault.release_request(p.scope());
    assert!(vault.capabilities(&p, &bearer).is_err());
    p.revoke();
    assert!(p
        .check_url(
            p.scope(),
            "https://api.example.invalid",
            &[IpAddr::from([8, 8, 8, 8])]
        )
        .is_err());
}

struct Service {
    origin: String,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    captures: Arc<Mutex<Vec<Value>>>,
}
impl Service {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let captures = Arc::new(Mutex::new(vec![]));
        let recorded = captures.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {_=stopping.cancelled()=>break,Some(_)=clients.join_next(),if !clients.is_empty()=>{},accepted=listener.accept()=>{let (mut socket,_)=accepted.unwrap();let captured=recorded.clone();clients.spawn(async move{
                    let mut header=vec![];let mut byte=[0u8;1];while !header.ends_with(b"\r\n\r\n"){if socket.read_exact(&mut byte).await.is_err(){return;}header.push(byte[0]);assert!(header.len()<65536);}let text=String::from_utf8(header).unwrap();let route=text.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_string();let length=text.lines().filter_map(|l|l.split_once(':')).find(|(n,_)|n.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.trim().parse::<usize>().unwrap()).unwrap_or(0);
                    let mut received=0usize;let mut hash=Sha256::new();let mut buffer=vec![0;65536];while received<length{let count=socket.read(&mut buffer[..(length-received).min(65536)]).await.unwrap();if count==0{return;}hash.update(&buffer[..count]);received+=count;}
                    captured.lock().unwrap().push(json!({"headers":text,"route":route,"length":received,"digest":hex::encode(hash.finalize())}));
                    if route.starts_with("/hang"){tokio::time::sleep(Duration::from_secs(30)).await;return;}
                    if route.starts_with("/slow-media"){let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\npartial").await;tokio::time::sleep(Duration::from_secs(30)).await;return;}
                    let (status,extra,body)=if route.starts_with("/redirect"){("302 Found","Location: http://127.0.0.1:1/escape\r\n",b"redirect".to_vec())}else if route.starts_with("/large"){("200 OK","",vec![b'x';32768])}else if route.starts_with("/image"){
                        let mut image=std::io::Cursor::new(vec![]);image::RgbImage::from_pixel(2,3,image::Rgb([20,30,40])).write_to(&mut image,image::ImageFormat::Png).unwrap();("200 OK","Content-Type: application/octet-stream\r\n",image.into_inner())
                    }else{("200 OK","",b"ok".to_vec())};
                    let response=format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",body.len());let _=socket.write_all(response.as_bytes()).await;let _=socket.write_all(&body).await;
                });}}
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });
        Self {
            origin,
            stop,
            task,
            captures,
        }
    }
    async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
fn broker(package: &VerifiedPackage, origin: &str) -> (tempfile::TempDir, HttpBroker) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let p = policy(package, "http", &[origin], true);
    let media = MediaAccess {
        broker: Arc::new(Mutex::new(
            MediaBroker::new(workspace, Duration::from_secs(60), None).unwrap(),
        )),
        scope: p.scope().clone(),
    };
    let broker = HttpBroker::new(
        p,
        Arc::new(Mutex::new(CredentialVault::new().unwrap())),
        media,
    )
    .unwrap();
    (directory, broker)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_loopback_http_uses_the_gate_and_never_follows_escaping_redirects() {
    let service = Service::start().await;
    let package = package(false);
    let (_directory, broker) = broker(&package, &service.origin);
    let control = Control::new(None);
    let key = broker
        .credentials
        .lock()
        .unwrap()
        .insert(
            &broker.policy,
            material(b"test-http-secret"),
            BTreeSet::from(["bearer".into()]),
            broker.policy.origins().to_vec(),
            Duration::from_secs(60),
        )
        .unwrap();
    let mut request = plan(&format!("{}/ok?secret=do-not-log-this", service.origin));
    request["generatedHeaders"] =
        json!([{"name":"Authorization","kind":"bearer","targetField":"authorization"}]);
    let response = broker
        .execute(
            &serde_json::from_value(request.clone()).unwrap(),
            Some(&key),
            &control,
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap()["payload"]["bytes"],
        json!([111, 107])
    );
    assert!(service.captures.lock().unwrap()[0]["headers"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("authorization: bearer test-http-secret"));
    let audit = serde_json::to_string(&broker.audit().unwrap()).unwrap();
    assert!(!audit.contains("test-http-secret"));
    assert!(!audit.contains("do-not-log-this"));
    let before = service.captures.lock().unwrap().len();
    assert!(broker
        .execute(
            &serde_json::from_value(request.clone()).unwrap(),
            Some("forged"),
            &control
        )
        .await
        .is_err());
    assert_eq!(service.captures.lock().unwrap().len(), before);
    request = plan(&format!("{}/redirect", service.origin));
    let redirect = broker
        .execute(&serde_json::from_value(request).unwrap(), None, &control)
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(redirect).unwrap()["status"], 302);
    assert_eq!(service.captures.lock().unwrap().len(), before + 1);
    let request: HostPortCallV2HttpRequest =
        serde_json::from_value(plan("http://127.0.0.1:1/escape")).unwrap();
    assert!(broker.execute(&request, None, &control).await.is_err());
    let mut large = plan(&format!("{}/large", service.origin));
    large["sink"]["maxBytes"] = json!(16);
    assert_eq!(
        serde_json::to_value(
            broker
                .execute(&serde_json::from_value(large).unwrap(), None, &control)
                .await
                .unwrap_err()
        )
        .unwrap()["code"],
        "BUDGET_EXCEEDED"
    );
    service.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_media_upload_download_and_cancel_are_streamed_and_scoped() {
    let service = Service::start().await;
    let package = package(false);
    let (directory, broker) = broker(&package, &service.origin);
    let control = Control::new(None);
    let source = directory.path().join("large-source.bin");
    let bytes = vec![0x58u8; 65536];
    let mut file = std::fs::File::create(&source).unwrap();
    use std::io::Write;
    for _ in 0..128 {
        file.write_all(&bytes).unwrap();
    }
    drop(file);
    let import_source = source.clone();
    let c = control.clone();
    let lease = broker
        .media
        .run(move |b, s| b.import_file(s, "upload", &import_source, "file", &c))
        .await
        .unwrap();
    let mut request = plan(&format!("{}/upload", service.origin));
    request["method"] = json!("POST");
    request["body"] = json!({"kind":"media","handle":lease.handle});
    broker
        .execute(&serde_json::from_value(request).unwrap(), None, &control)
        .await
        .unwrap();
    let capture = service.captures.lock().unwrap()[0].clone();
    assert_eq!(capture["length"], 8388608);
    let mut hash = Sha256::new();
    for _ in 0..128 {
        hash.update(&bytes);
    }
    assert_eq!(capture["digest"], hex::encode(hash.finalize()));
    let mut multipart = plan(&format!("{}/multipart", service.origin));
    multipart["method"] = json!("POST");
    multipart["body"] = json!({"kind":"multipart","parts":[{"kind":"field","name":"label","value":"example"},{"kind":"file","name":"file","fileName":lease.file_name,"handle":lease.handle,"mime":lease.stat.r#mime}]});
    broker
        .execute(
            &serde_json::from_value(multipart.clone()).unwrap(),
            None,
            &control,
        )
        .await
        .unwrap();
    let before = service.captures.lock().unwrap().len();
    multipart["body"]["parts"][1]["fileName"] = json!("incorrect.png");
    assert!(broker
        .execute(
            &serde_json::from_value(multipart.clone()).unwrap(),
            None,
            &control
        )
        .await
        .is_err());
    assert_eq!(service.captures.lock().unwrap().len(), before);
    multipart["body"]["parts"][1]["fileName"] = json!(lease.file_name);
    multipart["headers"] = json!([{"name":"Content-Type","value":"application/json"}]);
    assert!(broker
        .execute(&serde_json::from_value(multipart).unwrap(), None, &control)
        .await
        .is_err());
    assert_eq!(service.captures.lock().unwrap().len(), before);
    let mut request = plan(&format!("{}/image", service.origin));
    request["sink"] = json!({"kind":"media"});
    let response = serde_json::to_value(
        broker
            .execute(&serde_json::from_value(request).unwrap(), None, &control)
            .await
            .unwrap(),
    )
    .unwrap();
    let h = response["payload"]["handle"].as_str().unwrap().to_string();
    let stat = broker.media.run(move |b, s| b.info(s, &h)).await.unwrap();
    assert_eq!(stat.r#mime, "image/png");
    assert_eq!(stat.r#width, Some(2));
    assert_eq!(serde_json::to_value(stat.r#kind).unwrap(), "image");
    let mut request = plan(&format!("{}/slow-media", service.origin));
    request["sink"] = json!({"kind":"media"});
    request["timeoutMs"] = json!(100);
    assert_eq!(
        serde_json::to_value(
            broker
                .execute(&serde_json::from_value(request).unwrap(), None, &control)
                .await
                .unwrap_err()
        )
        .unwrap()["code"],
        "TIMEOUT"
    );
    let pending = Control::new(None);
    let cancel = pending.clone();
    let revoke = broker.policy.clone();
    let request: HostPortCallV2HttpRequest =
        serde_json::from_value(plan(&format!("{}/hang", service.origin))).unwrap();
    let future = broker.execute(&request, None, &pending);
    tokio::pin!(future);
    let interrupted = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    };
    let (result, _) = tokio::join!(&mut future, interrupted);
    assert_eq!(
        serde_json::to_value(result.unwrap_err()).unwrap()["code"],
        "CANCELLED"
    );
    // Dropping a pending media HTTP future must clean the output lease even
    // when the caller never polls its error result.
    {
        let local_control = Control::new(None);
        let mut slow = plan(&format!("{}/slow-media", service.origin));
        slow["timeoutMs"] = json!(0);
        slow["sink"] = json!({"kind":"media"});
        let slow: HostPortCallV2HttpRequest = serde_json::from_value(slow).unwrap();
        let pending = broker.execute(&slow, None, &local_control);
        tokio::pin!(pending);
        tokio::select! {result=&mut pending=>panic!("slow media unexpectedly finished: {result:?}"),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let count = std::fs::read_dir(directory.path().join("staging"))
                .unwrap()
                .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
                .count();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let revoking = Control::new(None);
    let future = broker.execute(&request, None, &revoking);
    let interrupted = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        revoke.revoke();
    };
    let (result, _) = tokio::join!(future, interrupted);
    assert_eq!(
        serde_json::to_value(result.unwrap_err()).unwrap()["code"],
        "PERMISSION_DENIED"
    );
    assert!(broker
        .execute(
            &request,
            None,
            &Control::new(Some(Instant::now() + Duration::from_secs(1)))
        )
        .await
        .is_err());
    let staged = std::fs::read_dir(directory.path().join("staging"))
        .unwrap()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .count();
    assert_eq!(staged, 0);
    service.close().await;
}
