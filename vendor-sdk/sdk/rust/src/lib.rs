#![no_std]
//! Public schema-generated wire types. Native validation is provided separately.
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod host;
pub mod guest;
pub mod submission_rejection;
pub use host::Host;
pub use guest::Adapter;

#[cfg(feature = "component-guest")]
#[doc(hidden)]
pub use dlmalloc as __dlmalloc;
#[cfg(feature = "component-guest")]
#[doc(hidden)]
pub mod __bindings {
    wit_bindgen::generate!({
        path: "../../wit/vendor-adapter.wit",
        world: "vendor-adapter",
        pub_export_macro: true,
        generate_unused_types: true,
        std_feature,
        additional_derives: [serde::Serialize],
    });
}

mod generated;
pub use generated::*;

/// Required nullable fields must be present even when their value is null.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn frontend_payload_roundtrip_and_strict_shapes() {
        let fixtures: Value = serde_json::from_str(include_str!("../../../examples/payloads.json")).unwrap();
        let request = fixtures["validSubmit"].clone();
        let typed: OperationSubmitV2 = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), request);
        let mut wrong = request.clone();
        wrong["host"] = json!("leak");
        assert!(serde_json::from_value::<OperationSubmitV2>(wrong).is_err());
        let mut wrong = request.clone();
        wrong["schemaVersion"] = json!(1);
        assert!(serde_json::from_value::<OperationSubmitV2>(wrong).is_err());
        let mut wrong = request;
        wrong.as_object_mut().unwrap().remove("uploads");
        assert!(serde_json::from_value::<OperationSubmitV2>(wrong).is_err());
    }

    #[test]
    fn invocation_and_response_wire_contracts() {
        let wire = json!({"protocolVersion":2,"operation":"submit","profileId":"dev",
            "capabilitySlot":"generic","requestId":"request-1","budgetMs":1000,
            "payloadJson":"{}","bindingId":"dev","negotiatedVersion":2,"enabledFeatures":[]});
        let typed: Invocation = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
        let error = json!({"protocolVersion":2,"operation":"submit","error":{
            "code":"INVALID_INPUT","message":"Invalid input","retryable":false,
            "vendorCode":null,"vendorRequestId":null}});
        let typed: Response = serde_json::from_value(error.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), error);
        let mut wrong = error.clone();
        wrong["dataJson"] = json!("{}");
        assert!(serde_json::from_value::<Response>(wrong).is_err());
        let mut wrong = error;
        wrong["error"].as_object_mut().unwrap().remove("vendorCode");
        assert!(serde_json::from_value::<Response>(wrong).is_err());
        assert!(serde_json::from_value::<SubmitResult>(json!({"status":"running","outputs":[],
            "data":null,"usage":null,"progress":null,"terminalFailure":null,"resumeToken":null})).is_err());
    }
}
