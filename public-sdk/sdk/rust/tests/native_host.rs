use bgx_adapter_v2_sdk::{*,host,guest};
use serde_json::{json,Value};

struct FakeHost { calls: Vec<Value> }
impl Host for FakeHost {
    fn call(&mut self,call:HostPortCallV2)->Result<Value,host::HostError>{
        let value=serde_json::to_value(call).unwrap();self.calls.push(value.clone());
        match value["method"].as_str().unwrap(){"host-log.log"=>Ok(Value::Null),"host-media.read"=>Ok(json!([0,127,255])),_=>Err(host::invalid_wire("unsupported test port"))}
    }
}
fn invocation()->Invocation{serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","profileId":"native","bindingId":"native",
    "capabilitySlot":"generic","requestId":"request","budgetMs":u64::MAX,"negotiatedVersion":2,"enabledFeatures":["stream-events-v1"],"payloadJson":"{}"})).unwrap()}

#[test]
fn native_author_entry_and_managed_ports_have_strict_wire_shapes(){
    struct Echo;
    impl Adapter for Echo{
        fn invoke(request:Invocation,host:&mut dyn Host)->Response{
            let _:()=host::call(host,"host-log.log",&json!({"level":"info","code":"native","message":"invoked"})).unwrap();
            let bytes:Vec<u8>=host::call(host,"host-media.read",&json!({"handle":"test-lease","offset":"0","maxBytes":3})).unwrap();
            guest::completed_response(&request,&json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"bytes":bytes},"usage":null})).unwrap()
        }
    }
    let mut host=FakeHost{calls:Vec::new()};let response=Echo::invoke(invocation(),&mut host);
    let wire=serde_json::to_value(response).unwrap();let result:Value=serde_json::from_str(wire["dataJson"].as_str().unwrap()).unwrap();
    assert_eq!(result["data"]["bytes"],json!([0,127,255]));assert_eq!(host.calls.len(),2);
    let invalid:Result<Value,_>=host::call(&mut host,"unregistered.port",&json!({}));assert!(invalid.is_err());assert_eq!(host.calls.len(),2);
}

#[test]
fn generated_component_wire_conversion_is_exhaustive_and_checks_constants(){
    let original=invocation();let component=wire_abi::to_invocation(original.clone()).unwrap();
    assert_eq!(component.budget_ms,u64::MAX);
    let restored=wire_abi::from_invocation(component).unwrap();assert_eq!(restored,original);
    let mut invalid=wire_abi::to_invocation(original).unwrap();invalid.protocol_version=1;
    assert!(wire_abi::from_invocation(invalid).is_err());
    let args:HostPortCallV2HostMediaWriteArgs=serde_json::from_value(json!({"handle":"h","bytes":[0,127,255]})).unwrap();
    let component=wire_abi::to_host_port_call_v2_host_media_write_args(args.clone()).unwrap();
    assert_eq!(wire_abi::from_host_port_call_v2_host_media_write_args(component).unwrap(),args);
    let model:Value=serde_json::from_str(wire_abi::BINDING_MODEL_JSON).unwrap();
    let binding=serde_json::to_value(wire_abi::to_invocation(invocation()).unwrap()).unwrap();
    let wire=wire_abi::decode_binding(&model,"Invocation",&binding).unwrap();
    assert_eq!(serde_json::from_value::<Invocation>(wire).unwrap(),invocation());
    let mut unknown=binding.clone();unknown["foreign_field"]=json!(true);assert!(wire_abi::decode_binding(&model,"Invocation",&unknown).is_err());
    let mut unknown=binding;unknown["operation"]=json!("V999");assert!(wire_abi::decode_binding(&model,"Invocation",&unknown).is_err());
}

#[test]
fn exported_memory_routines_preserve_overlap_and_byte_comparison(){
    use guest::memory::*;
    let mut bytes=[0,1,2,3,4,5,6,7];
    unsafe{
        move_bytes(bytes.as_mut_ptr().add(2),bytes.as_ptr(),6);assert_eq!(bytes,[0,1,0,1,2,3,4,5]);
        move_bytes(bytes.as_mut_ptr(),bytes.as_ptr().add(2),6);assert_eq!(bytes,[0,1,2,3,4,5,4,5]);
        set(bytes.as_mut_ptr().add(2),255,2);assert_eq!(bytes,[0,1,255,255,4,5,4,5]);
        assert_eq!(compare(bytes.as_ptr(),bytes.as_ptr(),8),0);
        let other=[0u8;8];assert!(compare(bytes.as_ptr(),other.as_ptr(),8)>0);
    }
}
