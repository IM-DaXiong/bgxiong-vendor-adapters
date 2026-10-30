use crate::guest;
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{wire_abi, Host, Invocation, ManifestV2Limits, Response},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use wasmtime::component::{
    types::{ComponentItem, Type},
    Component, Linker, Val,
};
use wasmtime::{Config, Engine, ResourceLimiter, Store};

#[derive(Debug, Clone)]
pub struct RuntimeFault {
    pub code: &'static str,
    pub message: String,
}
impl RuntimeFault {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for RuntimeFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RuntimeFault {}
type Outcome<T> = Result<T, RuntimeFault>;

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

struct ClockState {
    active: Option<Instant>,
    consumed: Duration,
}
pub struct ExecutionClock {
    state: Mutex<ClockState>,
    budget: Duration,
    cancel: Cancellation,
}
impl ExecutionClock {
    pub fn new(budget_ms: u64, cancel: Cancellation) -> Self {
        Self {
            state: Mutex::new(ClockState {
                active: None,
                consumed: Duration::ZERO,
            }),
            budget: Duration::from_millis(budget_ms),
            cancel,
        }
    }
    pub fn resume(&self) {
        self.state.lock().expect("clock mutex").active = Some(Instant::now());
    }
    pub fn pause(&self) {
        let mut state = self.state.lock().expect("clock mutex");
        if let Some(start) = state.active.take() {
            state.consumed += start.elapsed();
        }
    }
    pub fn expired(&self) -> bool {
        let state = self.state.lock().expect("clock mutex");
        state.consumed
            + state
                .active
                .map(|start| start.elapsed())
                .unwrap_or(Duration::ZERO)
            >= self.budget
    }
}
struct Watchdog {
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Watchdog {
    fn start(engine: Engine, clock: Arc<ExecutionClock>) -> Outcome<Self> {
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let handle = thread::Builder::new()
            .name("sdk-v2-execution-clock".into())
            .spawn(move || {
                while !finished.load(Ordering::SeqCst) {
                    if clock.cancel.is_cancelled() || clock.expired() {
                        engine.increment_epoch();
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
            .map_err(|_| {
                RuntimeFault::new("RESOURCE_UNAVAILABLE", "Cannot start execution supervisor")
            })?;
        Ok(Self {
            done,
            handle: Some(handle),
        })
    }
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Limits {
    memory: usize,
    used_memory: usize,
    table: usize,
    used_table: usize,
    hit: bool,
}
impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired
            .checked_sub(current)
            .ok_or_else(|| wasmtime::format_err!("invalid memory growth"))?;
        let next = self
            .used_memory
            .checked_add(delta)
            .ok_or_else(|| wasmtime::format_err!("memory accounting overflow"))?;
        if next > self.memory || maximum.is_some_and(|m| desired > m) {
            self.hit = true;
            wasmtime::bail!("guest memory limit exceeded");
        }
        self.used_memory = next;
        Ok(true)
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.hit = true;
        Err(error)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired
            .checked_sub(current)
            .ok_or_else(|| wasmtime::format_err!("invalid table growth"))?;
        let next = self
            .used_table
            .checked_add(delta)
            .ok_or_else(|| wasmtime::format_err!("table accounting overflow"))?;
        if next > self.table || maximum.is_some_and(|m| desired > m) {
            self.hit = true;
            wasmtime::bail!("guest table limit exceeded");
        }
        self.used_table = next;
        Ok(true)
    }
    fn instances(&self) -> usize {
        resource_policy()["limits"]["maxInstances"]
            .as_u64()
            .unwrap()
            .try_into()
            .expect("policy usize")
    }
    fn memories(&self) -> usize {
        resource_policy()["limits"]["maxMemories"]
            .as_u64()
            .unwrap()
            .try_into()
            .expect("policy usize")
    }
    fn tables(&self) -> usize {
        resource_policy()["limits"]["maxTables"]
            .as_u64()
            .unwrap()
            .try_into()
            .expect("policy usize")
    }
}
struct State {
    host: Box<dyn Host + Send>,
    clock: Arc<ExecutionClock>,
    limits: Limits,
    features: Vec<String>,
    offline: bool,
}
pub struct Runtime {
    engine: Engine,
    model: Arc<Value>,
    signatures: Arc<Value>,
    required_features: std::collections::BTreeMap<String, String>,
}
impl Runtime {
    pub fn new() -> Outcome<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true);
        // A worker compiles a component once. Avoid expensive optimizing JIT
        // startup; guest fuel/epoch/resource accounting remains enabled.
        config.cranelift_opt_level(wasmtime::OptLevel::None);
        let engine = Engine::new(&config)
            .map_err(|e| RuntimeFault::new("RESOURCE_UNAVAILABLE", e.to_string()))?;
        let catalog: Value =
            serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json"))
                .expect("feature catalog");
        let mut required_features = std::collections::BTreeMap::new();
        for feature in catalog["features"].as_array().unwrap() {
            for interface in feature["requiredImports"].as_array().unwrap() {
                required_features.insert(
                    interface.as_str().unwrap().into(),
                    feature["id"].as_str().unwrap().into(),
                );
            }
        }
        Ok(Self {
            engine,
            model: Arc::new(
                serde_json::from_str(wire_abi::BINDING_MODEL_JSON).expect("generated model"),
            ),
            signatures: Arc::new(
                serde_json::from_str(wire_abi::PORT_SIGNATURES_JSON).expect("generated signatures"),
            ),
            required_features,
        })
    }

    pub fn execute_file(
        &mut self,
        path: &Path,
        expected_hash: &str,
        request: Invocation,
        host: Box<dyn Host + Send>,
        limits: ManifestV2Limits,
        cancel: Cancellation,
    ) -> Outcome<Response> {
        let max = resource_policy()["limits"]["maxModuleBytes"]
            .as_u64()
            .unwrap();
        let file =
            File::open(path).map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
        if file
            .metadata()
            .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?
            .len()
            > max
        {
            return Err(RuntimeFault::new(
                "INVALID_PACKAGE",
                "Module exceeds public resource budget",
            ));
        }
        let mut bytes = Vec::new();
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
        self.execute_bytes(&bytes, expected_hash, request, host, limits, cancel)
    }

    pub fn execute_bytes(
        &mut self,
        bytes: &[u8],
        expected_hash: &str,
        request: Invocation,
        host: Box<dyn Host + Send>,
        limits: ManifestV2Limits,
        cancel: Cancellation,
    ) -> Outcome<Response> {
        if bytes.len() as u64
            > resource_policy()["limits"]["maxModuleBytes"]
                .as_u64()
                .unwrap()
            || hex::encode(Sha256::digest(bytes)) != expected_hash
        {
            return Err(RuntimeFault::new(
                "INVALID_PACKAGE",
                "Module digest or size differs from the selected package",
            ));
        }
        if cancel.is_cancelled() {
            return Err(RuntimeFault::new(
                "RESOURCE_UNAVAILABLE",
                "Local execution was cancelled before starting",
            ));
        }
        if request.r#request_id.is_empty()
            || request.r#binding_id.is_empty()
            || request.r#profile_id != request.r#binding_id
            || request.r#budget_ms == 0
            || limits.r#fuel == 0
            || limits.r#wall_ms == 0
        {
            return Err(RuntimeFault::new(
                "INVALID_INPUT",
                "Invocation binding or execution budget is invalid",
            ));
        }
        let component = Component::new(&self.engine, bytes)
            .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
        let linker = self.linker(&component)?;
        let wire = serde_json::to_value(&request).expect("typed invocation");
        let offline = wire["operation"] == "capabilities"
            || (wire["operation"] == "probe"
                && serde_json::from_str::<Value>(&request.r#payload_json)
                    .map_err(|_| RuntimeFault::new("INVALID_INPUT", "Invalid probe JSON"))?
                    ["kind"]
                    == "package_validation");
        let clock = Arc::new(ExecutionClock::new(
            request.r#budget_ms.min(limits.r#wall_ms),
            cancel.clone(),
        ));
        let memory = usize::try_from(limits.r#memory_bytes).map_err(|_| {
            RuntimeFault::new(
                "INVALID_INPUT",
                "Memory limit is outside the native address range",
            )
        })?;
        let mut store = Store::new(
            &self.engine,
            State {
                host,
                clock: clock.clone(),
                limits: Limits {
                    memory,
                    used_memory: 0,
                    table: resource_policy()["limits"]["maxTableElements"]
                        .as_u64()
                        .unwrap()
                        .try_into()
                        .expect("policy usize"),
                    used_table: 0,
                    hit: false,
                },
                features: wire["enabledFeatures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect(),
                offline,
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(limits.r#fuel)
            .map_err(|e| RuntimeFault::new("RESOURCE_UNAVAILABLE", e.to_string()))?;
        store.set_epoch_deadline(1);
        let _watchdog = Watchdog::start(self.engine.clone(), clock.clone())?;
        let wire_operation = wire["operation"].clone();
        let execution = (|| -> wasmtime::Result<Response> {
            clock.resume();
            let instance = linker.instantiate(&mut store, &component)?;
            clock.pause();
            let invoke = instance
                .get_func(&mut store, "invoke")
                .ok_or_else(|| wasmtime::format_err!("Missing invoke export"))?;
            let invoke_type = invoke.ty(&store);
            let params: Vec<_> = invoke_type.params().collect();
            let results: Vec<_> = invoke_type.results().collect();
            wasmtime::ensure!(
                params.len() == 1 && results.len() == 1,
                "Invoke signature differs"
            );
            guest::validate_type(&params[0].1, "Invocation", &self.model)?;
            guest::validate_type(&results[0], "Response", &self.model)?;
            let binding = serde_json::to_value(
                wire_abi::to_invocation(request)
                    .map_err(|_| wasmtime::format_err!("Invalid invocation ABI"))?,
            )?;
            let argument = guest::to_val(&params[0].1, &binding)?;
            let mut result = [Val::Bool(false)];
            clock.resume();
            invoke.call(&mut store, &[argument], &mut result)?;
            clock.pause();
            let binding = guest::from_val(&result[0])?;
            let wire = wire_abi::decode_binding(&self.model, "Response", &binding)
                .map_err(|_| wasmtime::format_err!("Invalid response ABI"))?;
            let response: Response = serde_json::from_value(wire)?;
            let response_wire = serde_json::to_value(&response)?;
            wasmtime::ensure!(
                response_wire["operation"] == wire_operation,
                "Response operation differs from invocation"
            );
            if let Some(data) = response_wire.get("dataJson") {
                let _: Value = serde_json::from_str(
                    data.as_str()
                        .ok_or_else(|| wasmtime::format_err!("Invalid dataJson"))?,
                )?;
            }
            if serde_json::to_vec(&response)?.len() as u64 > limits.r#output_bytes {
                return Err(RuntimeFault::new(
                    "RESOURCE_UNAVAILABLE",
                    "Response exceeds declared output budget",
                )
                .into());
            }
            Ok(response)
        })();
        clock.pause();
        if cancel.is_cancelled() {
            return Err(RuntimeFault::new(
                "RESOURCE_UNAVAILABLE",
                "Local execution was cancelled; remote cancellation is not confirmed",
            ));
        }
        if clock.expired() {
            return Err(RuntimeFault::new(
                "TIMEOUT",
                "Guest execution budget exhausted",
            ));
        }
        if store.data().limits.hit {
            return Err(RuntimeFault::new(
                "RESOURCE_UNAVAILABLE",
                "Guest memory or table budget exhausted",
            ));
        }
        execution.map_err(|e| {
            if let Some(fault) = e.downcast_ref::<RuntimeFault>() {
                return fault.clone();
            }
            let code = if matches!(
                e.downcast_ref::<wasmtime::Trap>(),
                Some(wasmtime::Trap::OutOfFuel)
            ) {
                "TIMEOUT"
            } else {
                "INVALID_PACKAGE"
            };
            RuntimeFault::new(code, e.to_string())
        })
    }

    fn linker(&self, component: &Component) -> Outcome<Linker<State>> {
        let protocol: Value = serde_json::from_str(include_str!(
            "../../../protocol/vendor-adapter-protocol.json"
        ))
        .expect("protocol");
        let (package, version) = protocol["witPackage"]
            .as_str()
            .unwrap()
            .rsplit_once('@')
            .unwrap();
        let mut imports = Vec::new();
        let component_type = component.component_type();
        for (name, item) in component_type.imports(&self.engine) {
            match item.ty {
                ComponentItem::Type(ty) => {
                    let expected = self
                        .model
                        .as_object()
                        .unwrap()
                        .iter()
                        .find(|(_, v)| v["witName"] == name)
                        .map(|(k, _)| k)
                        .ok_or_else(|| {
                            RuntimeFault::new("INVALID_PACKAGE", "Unknown root type import")
                        })?;
                    if !matches!(expected.as_str(), "Invocation" | "Response") {
                        return Err(RuntimeFault::new(
                            "INVALID_PACKAGE",
                            "Unpublished root type import",
                        ));
                    }
                    guest::validate_type(&ty, expected, &self.model)
                        .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
                }
                ComponentItem::ComponentInstance(interface) => {
                    let prefix = format!("{package}/");
                    let suffix = format!("@{version}");
                    let interface_name=name.strip_prefix(&prefix).and_then(|s|s.strip_suffix(&suffix)).ok_or_else(||RuntimeFault::new("PERMISSION_DENIED","Guest imports an unapproved interface; WASI file/socket access is unavailable"))?;
                    if interface_name != "types"
                        && !self
                            .signatures
                            .as_object()
                            .unwrap()
                            .keys()
                            .any(|key| key.split_once('.').unwrap().0 == interface_name)
                    {
                        return Err(RuntimeFault::new(
                            "PERMISSION_DENIED",
                            "Guest imports an unapproved host interface",
                        ));
                    }
                    let mut functions = Vec::new();
                    for (item_name, item) in interface.exports(&self.engine) {
                        match item.ty {
                            ComponentItem::Type(ty) => {
                                let expected = self
                                    .model
                                    .as_object()
                                    .unwrap()
                                    .iter()
                                    .find(|(_, v)| v["witName"] == item_name)
                                    .map(|(k, _)| k)
                                    .ok_or_else(|| {
                                        RuntimeFault::new(
                                            "INVALID_PACKAGE",
                                            "Unknown interface type",
                                        )
                                    })?;
                                guest::validate_type(&ty, expected, &self.model).map_err(|e| {
                                    RuntimeFault::new("INVALID_PACKAGE", e.to_string())
                                })?;
                            }
                            ComponentItem::ComponentFunc(function) => {
                                let method = format!("{interface_name}.{item_name}");
                                let signature = self.signatures.get(&method).ok_or_else(|| {
                                    RuntimeFault::new(
                                        "PERMISSION_DENIED",
                                        "Guest requests an unpublished host method",
                                    )
                                })?;
                                let params = function.params().collect::<Vec<_>>();
                                let results = function.results().collect::<Vec<_>>();
                                if params.len() != 1 || results.len() != 1 {
                                    return Err(RuntimeFault::new(
                                        "INVALID_PACKAGE",
                                        "Host method signature differs",
                                    ));
                                }
                                guest::validate_type(
                                    &params[0].1,
                                    signature["args"].as_str().unwrap(),
                                    &self.model,
                                )
                                .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
                                let Type::Result(result) = &results[0] else {
                                    return Err(RuntimeFault::new(
                                        "INVALID_PACKAGE",
                                        "Host method must return a result",
                                    ));
                                };
                                match (result.ok(), signature["result"].as_str()) {
                                    (None, None) => {}
                                    (Some(ty), Some(name)) => {
                                        guest::validate_type(&ty, name, &self.model).map_err(
                                            |e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()),
                                        )?
                                    }
                                    _ => {
                                        return Err(RuntimeFault::new(
                                            "INVALID_PACKAGE",
                                            "Host return type differs",
                                        ))
                                    }
                                };
                                guest::validate_type(
                                    &result.err().ok_or_else(|| {
                                        RuntimeFault::new(
                                            "INVALID_PACKAGE",
                                            "Host error type missing",
                                        )
                                    })?,
                                    signature["error"].as_str().unwrap(),
                                    &self.model,
                                )
                                .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
                                functions.push((item_name.to_owned(), method, results[0].clone()));
                            }
                            _ => {
                                return Err(RuntimeFault::new(
                                    "PERMISSION_DENIED",
                                    "Non-value imports are unavailable",
                                ))
                            }
                        }
                    }
                    imports.push((name.to_owned(), interface_name.to_owned(), functions));
                }
                _ => {
                    return Err(RuntimeFault::new(
                        "PERMISSION_DENIED",
                        "Direct core/component imports are unavailable",
                    ))
                }
            }
        }
        let mut linker = Linker::<State>::new(&self.engine);
        linker
            .define_unknown_imports_as_traps(component)
            .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
        linker.allow_shadowing(true);
        for (name, interface_name, functions) in imports {
            if functions.is_empty() {
                continue;
            }
            let mut root = linker.root();
            let mut instance = root
                .instance(&name)
                .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
            for (function, method, result_type) in functions {
                let model = self.model.clone();
                let feature = self.required_features.get(&interface_name).cloned();
                let network = interface_name == "host-http" || interface_name == "host-http-stream";
                instance
                    .func_new(&function, move |mut store, _signature, params, results| {
                        let clock = store.data().clock.clone();
                        clock.pause();
                        let output = (|| -> wasmtime::Result<Val> {
                            wasmtime::ensure!(
                                params.len() == 1 && results.len() == 1,
                                "Host frame signature differs"
                            );
                            let args = guest::from_val(&params[0])?;
                            let call = wire_abi::port_arguments_to_wire(&model, &method, &args)
                                .map_err(|_| wasmtime::format_err!("Invalid managed port arguments"))?;
                            let response = if network && store.data().offline {
                                Err(bgx_adapter_v2_ports::wire::host::invalid_wire(
                                    "Network is forbidden during capabilities/package validation",
                                ))
                            } else if feature
                                .as_ref()
                                .is_some_and(|f| !store.data().features.contains(f))
                            {
                                Err(bgx_adapter_v2_ports::wire::host::invalid_wire(
                                    "Required stream feature was not negotiated",
                                ))
                            } else {
                                store.data_mut().host.call(call)
                            };
                            let Type::Result(ty) = &result_type else {
                                wasmtime::bail!("Invalid host result type")
                            };
                            Ok(match response {
                                Ok(value) => {
                                    let binding = wire_abi::port_result_to_binding(&method, value)
                                        .map_err(|_| {
                                            wasmtime::format_err!("Invalid managed port result")
                                        })?;
                                    Val::Result(Ok(match ty.ok() {
                                        None => None,
                                        Some(ty) => Some(Box::new(guest::to_val(&ty, &binding)?)),
                                    }))
                                }
                                Err(error) => {
                                    let error = wire_abi::to_host_port_call_v2_host_error(error)
                                        .map_err(|_| {
                                            wasmtime::format_err!("Invalid managed host error")
                                        })?;
                                    let value = serde_json::to_value(error)?;
                                    Val::Result(Err(Some(Box::new(guest::to_val(
                                        &ty.err().ok_or_else(|| {
                                            wasmtime::format_err!("Host error type missing")
                                        })?,
                                        &value,
                                    )?))))
                                }
                            })
                        })();
                        clock.resume();
                        results[0] = output?;
                        Ok(())
                    })
                    .map_err(|e| RuntimeFault::new("INVALID_PACKAGE", e.to_string()))?;
            }
        }
        Ok(linker)
    }
}

pub fn error_response(request: &Invocation, fault: &RuntimeFault) -> Response {
    serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{"code":fault.code,"message":fault.message,"retryable":false,"vendorCode":null,"vendorRequestId":null}})).expect("runtime error uses the public response catalog")
}
