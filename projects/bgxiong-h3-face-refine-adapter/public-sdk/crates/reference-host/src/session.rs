//! Frozen request lifecycle and the checkpoint HostPort authorization boundary.
//! A recovered session can construct query/cancel context, never another submit.
use crate::{
    checkpoint::{fail, validate, CheckpointStore, Result},
    media::{LeaseScope, MediaBroker},
};
use bgx_adapter_v2_ports::{
    decimal_u64,
    wire::{CapabilityContractV2, OperationResultV2, OperationSubmitV2, OperationTaskRefV2},
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    capabilities::Binding,
    hash_bytes,
    media::{validate_host_media, MediaValidationContext, UploadProvenance},
    package::VerifiedPackage,
    validate_contract, validate_input, validate_result, validate_task_input,
    ValidatedOperationInput,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadProof {
    media_digest: String,
    reference: String,
    media_id: String,
    configuration_digest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frozen {
    contract: CapabilityContractV2,
    request: Value,
    uploads: Vec<UploadProof>,
}
fn value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("validated wire serialization")
}
fn digest(v: &Value) -> String {
    hash_bytes(&serde_json::to_vec(v).unwrap())
}
fn scrub(mut request: Value) -> Value {
    if let Some(media) = request["media"].as_array_mut() {
        for media in media {
            if let Some(object) = media.as_object_mut() {
                object.remove("handle");
            }
        }
    }
    request
}
fn invalid() -> crate::checkpoint::CheckpointError {
    fail(
        "INVALID_INPUT",
        "Frozen request or result differs from its validated contract",
    )
}
pub struct Session {
    checkpoint: Value,
    frozen: Frozen,
    submit_permit: bool,
}
pub struct QueryContext {
    pub input: OperationTaskRefV2,
    pub context: ValidatedOperationInput,
}
impl Session {
    /// Must complete before any submit side effect. Broker owns live facts;
    /// caller-provided lease facts cannot override them. Upload proofs are host-owned.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        store: &mut CheckpointStore,
        package: &VerifiedPackage,
        binding: &Binding,
        scope: &LeaseScope,
        context: &ValidatedOperationInput,
        host: &MediaValidationContext,
        media: &MediaBroker,
        configuration_ref: &str,
    ) -> Result<Self> {
        if context.transport_operation() != "submit"
            || scope.package != *package.identity()
            || binding.package() != package.identity()
            || scope.binding_id != binding.id()
            || scope.request_id.is_empty()
            || configuration_ref.is_empty()
            || context.request()["configuration"] != *binding.configuration()
            || host.package != scope.package
            || host.binding_id != scope.binding_id
            || host.request_id != scope.request_id
        {
            return Err(invalid());
        }
        let mut actual = media.validation_context(scope);
        actual.uploads_by_ref = host.uploads_by_ref.clone();
        validate_host_media(context, &actual).map_err(|_| invalid())?;
        let mut identities = vec![];
        for m in context.request()["media"].as_array().unwrap() {
            identities.push(value(
                &media
                    .identity(scope, m["handle"].as_str().unwrap())
                    .map_err(|_| invalid())?,
            ));
        }
        let uploads = context.request()["uploads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| {
                let reference = u["uploadRef"].as_str().unwrap();
                let proof = &actual.uploads_by_ref[reference];
                UploadProof {
                    media_digest: proof.media_digest.clone(),
                    reference: reference.into(),
                    media_id: proof.media_id.clone(),
                    configuration_digest: proof.configuration_digest.clone(),
                }
            })
            .collect();
        let frozen = Frozen {
            contract: serde_json::from_value(context.contract().clone()).map_err(|_| invalid())?,
            request: scrub(context.request().clone()),
            uploads,
        };
        let request = context.request();
        let operation = context.operation();
        let checkpoint = json!({"schemaVersion":2,"revision":"1","requestId":scope.request_id,
            "package":package.identity(),"bindingId":binding.id(),"modelId":request["modelId"],
            "capabilityKey":request["capabilityKey"],"operationId":request["operationId"],"operationVersion":request["operationVersion"],
            "inputSchemaDigest":operation["inputSchema"]["digest"],"outputSchemaDigest":operation["outputSchema"]["digest"],
            "resumeSchemaDigest":operation["resumeSchema"].get("digest"),"configurationDigest":digest(binding.configuration()),
            "configurationRef":configuration_ref,"requestDigest":digest(&value(&frozen)),"state":"prepared",
            "vendorTaskId":null,"resumeToken":null,"mediaIdentities":identities,"result":null});
        store.create(&checkpoint, &value(&frozen))?;
        Ok(Self {
            checkpoint,
            frozen,
            submit_permit: true,
        })
    }
    /// Loads exactly the original package and declaration. A newer package is
    /// never silently substituted; the caller may explicitly supply the old one.
    pub fn recover(
        store: &CheckpointStore,
        request_id: &str,
        package: &VerifiedPackage,
        binding: &Binding,
        configuration_ref: &str,
    ) -> Result<Self> {
        let saved = store
            .load(request_id)?
            .ok_or_else(|| fail("RESOURCE_UNAVAILABLE", "Original checkpoint is missing"))?;
        let checkpoint = saved.checkpoint;
        if checkpoint["package"] != value(package.identity())
            || binding.package() != package.identity()
            || checkpoint["bindingId"] != binding.id()
            || checkpoint["configurationRef"] != configuration_ref
            || checkpoint["configurationDigest"] != digest(binding.configuration())
        {
            return Err(fail("RESOURCE_UNAVAILABLE", "Pinned package, binding or configuration is unavailable; query cannot use a replacement"));
        }
        let frozen: Frozen = serde_json::from_value(saved.frozen).map_err(|_| invalid())?;
        validate_contract(&frozen.contract).map_err(|_| invalid())?;
        let c = value(&frozen.contract);
        let op = c["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| {
                op["id"] == checkpoint["operationId"]
                    && op["version"] == checkpoint["operationVersion"]
            })
            .ok_or_else(invalid)?;
        if digest(&value(&frozen)) != checkpoint["requestDigest"]
            || frozen.request["configuration"] != *binding.configuration()
            || c["capabilityKey"] != checkpoint["capabilityKey"]
            || op["inputSchema"]["digest"] != checkpoint["inputSchemaDigest"]
            || op["outputSchema"]["digest"] != checkpoint["outputSchemaDigest"]
            || op["resumeSchema"].get("digest").unwrap_or(&Value::Null)
                != &checkpoint["resumeSchemaDigest"]
            || [
                "modelId",
                "capabilityKey",
                "operationId",
                "operationVersion",
                "inputSchemaDigest",
            ]
            .iter()
            .any(|k| frozen.request[*k] != checkpoint[*k])
        {
            return Err(invalid());
        }
        Ok(Self {
            checkpoint,
            frozen,
            submit_permit: false,
        })
    }
    /// Validate a retained root submit against its original frozen request.
    /// This read-only context does not restore a submit permit or mutate a checkpoint.
    pub fn retained_submission_context(&self, request: &OperationSubmitV2) -> Result<ValidatedOperationInput> {
        if scrub(value(request)) != self.frozen.request {
            return Err(invalid());
        }
        validate_input(&self.frozen.contract, request).map_err(|_| invalid())
    }
    pub fn checkpoint(&self) -> &Value {
        &self.checkpoint
    }
    /// Commit uncertainty BEFORE invoking submit. A crash at any later point
    /// cannot authorize another submit, even if the remote accepted no work.
    pub fn begin_submit(&mut self, store: &mut CheckpointStore) -> Result<()> {
        if !self.submit_permit || self.checkpoint["state"] != "prepared" {
            return Err(fail(
                "ACCEPTANCE_UNKNOWN",
                "Submission cannot be repeated from this checkpoint",
            ));
        }
        let mut next = self.next()?;
        next["state"] = json!("acceptance_unknown");
        self.commit(store, next)?;
        self.submit_permit = false;
        Ok(())
    }
    fn next(&self) -> Result<Value> {
        let revision = decimal_u64(self.checkpoint["revision"].as_str().unwrap())
            .map_err(|_| invalid())?
            .checked_add(1)
            .ok_or_else(|| fail("BUDGET_EXCEEDED", "Checkpoint revision exhausted"))?;
        let mut next = self.checkpoint.clone();
        next["revision"] = json!(revision.to_string());
        Ok(next)
    }
    fn candidate(
        &self,
        context: &ValidatedOperationInput,
        result: &OperationResultV2,
        media: Option<(&MediaBroker, &LeaseScope)>,
    ) -> Result<Value> {
        let state = self.checkpoint["state"].as_str().unwrap();
        if !matches!(state, "acceptance_unknown" | "accepted")
            || context.contract() != &value(&self.frozen.contract)
            || scrub(context.request().clone()) != self.frozen.request
            || context.transport_operation()
                != if state == "accepted" {
                    "query"
                } else {
                    "submit"
                }
        {
            return Err(invalid());
        }
        validate_result(context, result).map_err(|_| invalid())?;
        let result = value(result);
        let mut ledger = bgx_adapter_v2_validator::result::PartialOutputLedger::default();
        let previous = self.checkpoint["result"]
            .get("outputs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let outputs = result
            .get("outputs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        ledger.observe(previous).map_err(|_| invalid())?;
        ledger.observe(outputs).map_err(|_| invalid())?;
        if ledger.outputs().len() != outputs.len() {
            return Err(fail(
                "INVALID_INPUT",
                "Previously delivered outputs cannot be omitted from a later snapshot",
            ));
        }
        let mut next = self.next()?;
        for output in outputs.iter().filter(|o| o["kind"] == "media") {
            let (broker, scope) = media.ok_or_else(|| {
                fail(
                    "PERMISSION_DENIED",
                    "Media result requires live broker validation and durable identity",
                )
            })?;
            bgx_adapter_v2_validator::media::validate_leased_media(
                &output["media"],
                &broker.validation_context(scope),
            )
            .map_err(|_| invalid())?;
            let identity = value(
                &broker
                    .identity(scope, output["media"]["handle"].as_str().unwrap())
                    .map_err(|_| invalid())?,
            );
            let identities = next["mediaIdentities"].as_array_mut().unwrap();
            if let Some(prior) = identities.iter().find(|id| id["id"] == identity["id"]) {
                if prior != &identity {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Delivered media identity cannot change content",
                    ));
                }
            } else {
                identities.push(identity);
            }
        }
        let terminal = result["kind"] == "completed"
            || matches!(
                result["status"].as_str(),
                Some("succeeded" | "failed" | "cancelled" | "expired")
            );
        if state == "acceptance_unknown" {
            if result["kind"] == "accepted" {
                next["vendorTaskId"] = result["vendorTaskId"].clone();
            } else if result["kind"] == "completed" {
                next["vendorTaskId"] = result["vendorTaskId"].clone();
            } else {
                return Err(invalid());
            }
        }
        next["state"] = json!(if terminal { "terminal" } else { "accepted" });
        next["resumeToken"] = result["resumeToken"].clone();
        next["result"] = result;
        Ok(next)
    }
    fn commit(&mut self, store: &mut CheckpointStore, next: Value) -> Result<()> {
        store.update(&self.checkpoint, &next)?;
        self.checkpoint = next;
        Ok(())
    }
    /// Accepted identity/token/result are committed together before returning.
    pub fn record_result(
        &mut self,
        store: &mut CheckpointStore,
        context: &ValidatedOperationInput,
        result: &OperationResultV2,
    ) -> Result<()> {
        self.commit(store, self.candidate(context, result, None)?)
    }
    pub fn record_result_with_media(
        &mut self,
        store: &mut CheckpointStore,
        context: &ValidatedOperationInput,
        result: &OperationResultV2,
        media: &MediaBroker,
        scope: &LeaseScope,
    ) -> Result<()> {
        self.commit(
            store,
            self.candidate(context, result, Some((media, scope)))?,
        )
    }
    /// Public checkpoint.write semantics; immutable anchors and state are checked
    /// against the current validated invocation, never accepted on schema alone.
    pub fn write(
        &mut self,
        store: &mut CheckpointStore,
        request_id: &str,
        expected_revision: &str,
        checkpoint: Value,
        context: &ValidatedOperationInput,
    ) -> Result<String> {
        self.write_validated(
            store,
            request_id,
            expected_revision,
            checkpoint,
            context,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write_with_media(
        &mut self,
        store: &mut CheckpointStore,
        request_id: &str,
        expected_revision: &str,
        checkpoint: Value,
        context: &ValidatedOperationInput,
        media: &MediaBroker,
        scope: &LeaseScope,
    ) -> Result<String> {
        self.write_validated(
            store,
            request_id,
            expected_revision,
            checkpoint,
            context,
            Some((media, scope)),
        )
    }
    fn write_validated(
        &mut self,
        store: &mut CheckpointStore,
        request_id: &str,
        expected_revision: &str,
        checkpoint: Value,
        context: &ValidatedOperationInput,
        media: Option<(&MediaBroker, &LeaseScope)>,
    ) -> Result<String> {
        if checkpoint["requestId"] != request_id || self.checkpoint["requestId"] != request_id {
            return Err(fail(
                "PERMISSION_DENIED",
                "Checkpoint belongs to another request",
            ));
        }
        validate(&checkpoint)?;
        if self.checkpoint["revision"] != expected_revision {
            return Err(fail(
                "CHECKPOINT_CONFLICT",
                "Checkpoint revision has changed",
            ));
        }
        let result: OperationResultV2 =
            serde_json::from_value(checkpoint["result"].clone()).map_err(|_| invalid())?;
        let candidate = self.candidate(context, &result, media)?;
        if candidate != checkpoint {
            return Err(invalid());
        }
        self.commit(store, candidate)?;
        Ok(self.checkpoint["revision"].as_str().unwrap().into())
    }
    pub fn query_context(
        &self,
        media: &mut MediaBroker,
        scope: &LeaseScope,
        control: &Control,
    ) -> Result<QueryContext> {
        match self.checkpoint["state"].as_str().unwrap() {
            "acceptance_unknown" => {
                return Err(fail(
                    "ACCEPTANCE_UNKNOWN",
                    "Remote acceptance is unknown; automatic resubmission is forbidden",
                ))
            }
            "prepared" => {
                return Err(fail(
                    "INVALID_INPUT",
                    "Submission has not started; no remote task can be queried",
                ))
            }
            "terminal" => {
                return Err(fail(
                    "INVALID_INPUT",
                    "Task already has a terminal checkpoint",
                ))
            }
            _ => {}
        }
        if value(&scope.package) != self.checkpoint["package"]
            || scope.binding_id != self.checkpoint["bindingId"]
            || scope.request_id.is_empty()
        {
            return Err(invalid());
        }
        let c = value(&self.frozen.contract);
        let op = c["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| {
                op["id"] == self.checkpoint["operationId"]
                    && op["version"] == self.checkpoint["operationVersion"]
            })
            .ok_or_else(invalid)?;
        if op["execution"]["mode"] != "asynchronous" {
            return Err(fail(
                "UNSUPPORTED_FEATURE",
                "Only asynchronous tasks have a query lifecycle",
            ));
        }
        let vendor_id = self.checkpoint["vendorTaskId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;
        let mut request = self.frozen.request.clone();
        let mut acquired = vec![];
        let result = (|| {
            let identities = self.checkpoint["mediaIdentities"]
                .as_array()
                .ok_or_else(invalid)?;
            let items = request["media"].as_array_mut().ok_or_else(invalid)?;
            for m in items {
                let identity = identities
                    .iter()
                    .find(|id| id["id"] == m["id"] && id["kind"] == m["kind"])
                    .ok_or_else(invalid)?;
                let leased = media
                    .rehydrate(
                        scope,
                        &serde_json::from_value(identity.clone()).map_err(|_| invalid())?,
                        control,
                    )
                    .map_err(|_| {
                        fail(
                            "RESOURCE_UNAVAILABLE",
                            "Original media is missing or changed",
                        )
                    })?;
                acquired.push(leased.handle.clone());
                m["handle"] = json!(leased.handle);
            }
            let request: OperationSubmitV2 =
                serde_json::from_value(request).map_err(|_| invalid())?;
            let original =
                validate_input(&self.frozen.contract, &request).map_err(|_| invalid())?;
            let mut host = media.validation_context(scope);
            for proof in &self.frozen.uploads {
                host.uploads_by_ref.insert(
                    proof.reference.clone(),
                    UploadProvenance {
                        media_digest: proof.media_digest.clone(),
                        package: scope.package.clone(),
                        binding_id: scope.binding_id.clone(),
                        configuration_digest: proof.configuration_digest.clone(),
                        media_id: proof.media_id.clone(),
                    },
                );
            }
            validate_host_media(&original, &host).map_err(|_| invalid())?;
            let input: OperationTaskRefV2 = serde_json::from_value(json!({"schemaVersion":2,"modelId":self.checkpoint["modelId"],"capabilityKey":self.checkpoint["capabilityKey"],"operationId":self.checkpoint["operationId"],"operationVersion":self.checkpoint["operationVersion"],"vendorTaskId":vendor_id,"resumeToken":self.checkpoint["resumeToken"],"configuration":self.frozen.request["configuration"]})).map_err(|_| invalid())?;
            let context = validate_task_input(&original, &input, "query", vendor_id)
                .map_err(|_| invalid())?;
            // Revalidate saved nonterminal result against the same frozen query contract.
            if self.checkpoint["result"]["kind"] == "accepted" {
                validate_result(
                    &original,
                    &serde_json::from_value(self.checkpoint["result"].clone())
                        .map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?;
            } else {
                validate_result(
                    &context,
                    &serde_json::from_value(self.checkpoint["result"].clone())
                        .map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?;
            }
            Ok(QueryContext { input, context })
        })();
        if result.is_err() {
            for handle in acquired {
                media
                    .release(scope, &handle)
                    .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Recovery media cleanup failed"))?;
            }
        }
        result
    }
}
