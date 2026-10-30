//! Platform-neutral native validation. No filesystem, network or application state.
pub mod context;
mod input;
pub mod schema_registry;
pub mod identity;
pub mod package;
pub mod capabilities;
pub mod media;
pub mod interaction;
pub mod result;
pub mod usage;

pub use context::{ContractError, ValidatedOperationInput};
pub use result::{validate_result, validate_response, validate_task_input};
pub use schema_registry::{SchemaRegistry, hash_bytes};

pub fn validate_contract(contract: &bgx_adapter_v2_sdk::CapabilityContractV2) -> Result<(), ContractError> {
    input::validate_contract(contract)?;
    media::validate_slots(&serde_json::to_value(contract).unwrap())
}
pub fn validate_configuration(contract:&bgx_adapter_v2_sdk::CapabilityContractV2,configuration:&serde_json::Value)->Result<(),ContractError>{
    validate_contract(contract)?;
    let c=serde_json::to_value(contract).map_err(|_|ContractError::new("INVALID_PACKAGE","/","contract cannot be encoded"))?;
    input::registry(&c)?.validate(c["configurationSchema"]["id"].as_str().unwrap(),configuration)
}

/// Schema and reference checks. Live leases and source geometry additionally
/// require validate_host_media before a host dispatches network work.
pub fn validate_input(contract: &bgx_adapter_v2_sdk::CapabilityContractV2,
                      request: &bgx_adapter_v2_sdk::OperationSubmitV2) -> Result<ValidatedOperationInput, ContractError> {
    let context = input::validate_input(contract, request)?;
    media::validate_slots(context.contract())?;
    media::validate_references(&context)?;
    interaction::validate_shapes(&context)?;
    Ok(context)
}

pub fn validate_input_with_media(contract: &bgx_adapter_v2_sdk::CapabilityContractV2,
                                request: &bgx_adapter_v2_sdk::OperationSubmitV2,
                                host: &media::MediaValidationContext) -> Result<ValidatedOperationInput, ContractError> {
    let context = validate_input(contract, request)?;
    media::validate_host_media(&context, host)?;
    Ok(context)
}

/// Optional UI advice; callers must not use this as an execution gate.
pub use input::ui_diagnostics;
