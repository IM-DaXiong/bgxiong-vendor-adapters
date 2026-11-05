use super::*;

#[derive(Clone, Copy)]
pub(super) enum Phase { BeforeSubmission, ServiceRejected, Unknown }

pub(super) fn record(h: &mut dyn Host, request_id: &str, phase: Phase, failure: &Failure) -> Result<()> {
    let basis = match phase {
        Phase::BeforeSubmission => "before_submission",
        Phase::ServiceRejected => "service_rejected",
        Phase::Unknown => return Ok(()),
    };
    let row = port(h, "host-checkpoint.read", json!({"requestId":request_id}))?;
    let revision = row["revision"].as_str().ok_or_else(|| fail("INTERNAL_ERROR", "submission rejection checkpoint missing"))?;
    let raw = row["valueJson"].as_str().ok_or_else(|| fail("INTERNAL_ERROR", "submission rejection checkpoint value missing"))?;
    let checkpoint: Value = serde_json::from_str(raw).map_err(|_| fail("INTERNAL_ERROR", "submission rejection checkpoint invalid"))?;
    if checkpoint["requestId"].as_str() != Some(request_id) {
        return Err(fail("INTERNAL_ERROR", "submission rejection request identity mismatch"));
    }
    let evidence = json!({"adapterCode":failure.code,"phase":basis,"diagnostic":failure.message});
    let checkpoint = bgx_adapter_v2_sdk::submission_rejection::checkpoint_candidate(&checkpoint, basis, &failure.message, &evidence)
        .map_err(|e| fail("INTERNAL_ERROR", e))?;
    port(h, "host-checkpoint.write", json!({"requestId":request_id,"expectedRevision":revision,"valueJson":checkpoint.to_string()}))?;
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    struct MemoryHost { calls: usize, written: Option<Value>, reject_write: bool }
    impl Host for MemoryHost {
        fn call(&mut self, call: bgx_adapter_v2_sdk::HostPortCallV2) -> core::result::Result<Value, host::HostError> {
            self.calls += 1;
            let v = serde_json::to_value(call).unwrap();
            if v["method"] == "host-checkpoint.read" {
                return Ok(json!({"revision":"2","valueJson":json!({"revision":"2","state":"acceptance_unknown","requestId":"req","requestDigest":"abc","vendorTaskId":null}).to_string()}));
            }
            if self.reject_write { return Err(host::invalid_wire("CAS conflict")); }
            self.written = Some(serde_json::from_str(v["args"]["valueJson"].as_str().unwrap()).unwrap());
            Ok(json!({"revision":"3"}))
        }
    }
    #[test] fn known_rejection_writes_fact_unknown_does_not_and_cas_failure_propagates() {
        let mut h=MemoryHost{calls:0,written:None,reject_write:false};
        record(&mut h,"req",Phase::Unknown,&invalid("x")).unwrap(); assert_eq!(h.calls,0);
        record(&mut h,"req",Phase::BeforeSubmission,&invalid("selection mismatch")).unwrap();
        assert_eq!(h.written.as_ref().unwrap()["state"],"rejected");
        assert_eq!(h.written.as_ref().unwrap()["submissionRejection"]["basis"],"before_submission");
        h.reject_write=true;
        assert!(record(&mut h,"req",Phase::ServiceRejected,&invalid("validation rejected")).is_err());
    }
}
