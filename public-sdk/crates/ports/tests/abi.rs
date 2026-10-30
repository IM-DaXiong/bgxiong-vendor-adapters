use bgx_adapter_v2_ports::*;
use bgx_adapter_v2_sdk::*;
use serde_json::{json,Value};
use wasmtime::{Engine,Config,Store};
use wasmtime::component::{Component,Linker,Val};

#[test]
fn published_wit_instantiates_only_when_required_imports_are_linked(){
    let sdk=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let check=std::process::Command::new("python").arg(sdk.join("tools/generate_wit.py")).arg("--check").output().unwrap();
    assert!(check.status.success(),"{}",String::from_utf8_lossy(&check.stderr));
    let wit=include_str!("../../../wit/vendor-adapter.wit");
    let mut resolve=wit_parser::Resolve::default();
    let package=resolve.push_str("vendor-adapter.wit",wit).unwrap();
    let world=resolve.select_world(package,Some("vendor-adapter")).unwrap();
    let interfaces=resolve.worlds[world].imports.values().filter(|item|matches!(item,wit_parser::WorldItem::Interface{..})).count();
    assert_eq!(interfaces,9); // eight host interfaces plus shared types (world type aliases are separate imports)
    let mut module=wit_component::dummy_module(&resolve,world,wit_parser::Mangling::Standard32);
    wit_component::embed_component_metadata(&mut module,&resolve,world,wit_component::StringEncoding::UTF8).unwrap();
    let encoded=wit_component::ComponentEncoder::default().module(&module).unwrap().validate(true).encode().unwrap();
    let mut config=Config::new();config.wasm_component_model(true);
    let engine=Engine::new(&config).unwrap();let component=Component::new(&engine,&encoded).unwrap();
    let empty=Linker::<()>::new(&engine);let mut store=Store::new(&engine,());
    assert!(empty.instantiate(&mut store,&component).is_err());
    let mut complete=Linker::<()>::new(&engine);complete.define_unknown_imports_as_traps(&component).unwrap();
    complete.instantiate(&mut store,&component).unwrap();
    // This is only an ABI fixture; no executable business case is certified here.
}

#[test]
fn component_boundary_preserves_bytes_u64_option_and_enum(){
    let bytes=wat::parse_str(r#"(component
        (type $host (instance
            (type $choice (enum "v0" "v1"))
            (export "choice" (type $choice-export (eq $choice)))
            (type $payload (tuple (list u8) u64 (option string) $choice-export))
            (export "payload" (type $payload-export (eq $payload)))
            (export "echo" (func (param "value" $payload-export) (result $payload-export)))
        ))
        (import "host" (instance $host-instance (type $host)))
        (alias export $host-instance "choice" (type $choice))
        (alias export $host-instance "payload" (type $payload))
        (alias export $host-instance "echo" (func $echo))
        (export "choice" (type $choice))
        (export "payload" (type $payload))
        (core module $memory-module
            (memory (export "memory") 1)
            (global $next (mut i32) (i32.const 4096))
            (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local $address i32)
                global.get $next local.set $address
                global.get $next local.get 3 i32.add i32.const 15 i32.add i32.const -16 i32.and global.set $next
                local.get $address)
        )
        (core instance $memory-instance (instantiate $memory-module))
        (alias core export $memory-instance "memory" (core memory $memory))
        (alias core export $memory-instance "realloc" (core func $realloc))
        (core func $lower (canon lower (func $echo) (memory $memory) (realloc $realloc)))
        (core module $bridge-module
            (import "host" "echo" (func $lower (param i32 i32 i64 i32 i32 i32 i32 i32)))
            (func (export "echo") (param i32 i32 i64 i32 i32 i32 i32) (result i32)
                local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 local.get 5 local.get 6
                i32.const 128 call $lower i32.const 128)
        )
        (core instance $bridge (instantiate $bridge-module (with "host" (instance (export "echo" (func $lower))))))
        (alias core export $bridge "echo" (core func $roundtrip))
        (func $roundtrip (param "value" $payload) (result $payload)
            (canon lift (core func $roundtrip) (memory $memory) (realloc $realloc)))
        (export "echo" (func $roundtrip))
    )"#).unwrap();
    let mut config=Config::new();config.wasm_component_model(true);let engine=Engine::new(&config).unwrap();
    let component=Component::new(&engine,&bytes).unwrap();let mut linker=Linker::<()>::new(&engine);
    linker.root().instance("host").unwrap().func_new("echo",|_,_signature,params,results|{results[0]=params[0].clone();Ok(())}).unwrap();
    let mut store=Store::new(&engine,());let instance=linker.instantiate(&mut store,&component).unwrap();let echo=instance.get_func(&mut store,"echo").unwrap();
    let input=Val::Tuple(vec![Val::List(vec![Val::U8(0),Val::U8(127),Val::U8(255)]),Val::U64(u64::MAX),
        Val::Option(Some(Box::new(Val::String("boundary".into())))),Val::Enum("v1".into())]);
    let mut results=[Val::Bool(false)];echo.call(&mut store,&[input.clone()],&mut results).unwrap();
    assert_eq!(results[0],input);
}

struct Echo;
impl HostPorts for Echo{
    fn call(&mut self,call:HostPortCallV2)->PortResult<Value>{
        let value=serde_json::to_value(call).unwrap();
        assert_eq!(value["method"],"host-media.read");assert_eq!(value["args"]["offset"],u64::MAX.to_string());
        Ok(json!([0,127,255]))
    }
}

#[test]
fn native_dto_and_checked_decimal_roundtrip(){
    let mut host=Echo;
    let args:HostPortCallV2HostMediaReadArgs=serde_json::from_value(json!({"handle":"lease","offset":u64::MAX.to_string(),"maxBytes":3})).unwrap();
    let bytes=host.host_media_read(args).unwrap();assert_eq!(bytes,vec![0u8,127,255]);
    assert_eq!(decimal_u64(&u64::MAX.to_string()).unwrap(),u64::MAX);
    for invalid in ["18446744073709551616","-1","01","+1",""]{assert!(decimal_u64(invalid).is_err());}
    assert!(serde_json::from_value::<HostPortCallV2>(json!({"method":"host-media.write","args":{"handle":"h","bytes":[256]}})).is_err());
    assert!(serde_json::from_value::<HostPortCallV2>(json!({"method":"unknown-port","args":{}})).is_err());
}
