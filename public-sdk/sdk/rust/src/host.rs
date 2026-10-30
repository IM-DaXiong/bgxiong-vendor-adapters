use alloc::string::String;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value,json};
use crate::{HostPortCallV2,HostPortCallV2HostError};

pub type HostError = HostPortCallV2HostError;

/// One schema-generated managed port request. Implementations must enforce live
/// permissions, ownership, budgets and cancellation before performing effects.
pub trait Host {
    fn call(&mut self, call: HostPortCallV2) -> Result<Value,HostError>;
}

pub fn invalid_wire(message: &str) -> HostError {
    serde_json::from_value(json!({"code":"INVALID_INPUT","message":message})).expect("published host error shape")
}

/// Typed helper for authors; unknown methods/fields fail the generated decoder.
pub fn call<A:Serialize,R:DeserializeOwned>(host:&mut dyn Host,method:&str,args:&A)->Result<R,HostError>{
    let call=serde_json::from_value(json!({"method":method,"args":args})).map_err(|_|invalid_wire("Invalid managed port request"))?;
    let output=host.call(call)?;
    serde_json::from_value(output).map_err(|_|invalid_wire("Invalid managed port result"))
}

#[cfg(feature="component-guest")]
pub(crate) struct ComponentHost;
#[cfg(feature="component-guest")]
impl Host for ComponentHost {
    fn call(&mut self,call:HostPortCallV2)->Result<Value,HostError>{crate::generated::wire_abi::call_component(call)}
}

pub fn diagnostic(error:&HostError)->String{error.r#message.clone()}
