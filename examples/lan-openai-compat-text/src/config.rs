//! # AI_AUTHOR_GUIDE
//!
//! Guest crate for any OpenAI-compatible LAN HTTP origin. Host `src-tauri/**`
//! must not name this crate, Ollama, or "lan-llama".
//!
//! Two credential profiles, **same wasm**:
//! 1. Third-party engine origin (`endpointBaseUrl` + optional Bearer).
//! 2. This product LAN gateway on port **18380** (not llama 18080-18089).
//!
//! ## Consts to edit
//! | Symbol | Default | Change when |
//! |---|---|---|
//! | `EXAMPLE_BASE_URL` | `http://192.168.0.10:11434` | Document specimen only. Runtime never falls back here. |
//! | `EXAMPLE_MODEL_ID` | `example-lan-llm` | Must match `/v1/models` id or probe list. Sync manifest. |
//! | `MODELS_PATH` | `/v1/models` | Non-OpenAI prefix |
//! | `CHAT_COMPLETIONS_PATH` | `/v1/chat/completions` | Same |
//! | `EXAMPLE_TIMEOUT_MS` | `600000` | Slow local models (do not copy an 8h wall clock) |
//! | `AUTH_USES_BEARER` | `true` | Set `false` for engines that reject Authorization |
//!
//! Do not put unspecified bind hosts in the origin. Do not point this crate
//! at this product llama loopback pool. Do not bake a LAN Bearer into source. Empty Key + bearer
//! is a host `CREDENTIAL_MISSING`.
//!
//! `thinking==true` is a hard error unless you map it and delete the reject.
//! `responseFormatJson=true` maps to OpenAI `response_format.json_object`;
//! unsupported engines surface vendor HTTP errors (do not rewrite to plain).
//!
//! ## Import root
//! Select the folder with `Cargo.toml` + `manifest.json` + `wit/` +
//! `vendor-sdk/` + `src/`. Do not select `src/` alone.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};

pub const PLUGIN_ID: &str = "local.example.lan-openai-compat-text";
pub const PLUGIN_VERSION: &str = "0.1.0";
pub const EXAMPLE_MODEL_ID: &str = "example-lan-llm";

/// Specimen origin shape (Ollama-like port). Not a runtime fallback.
pub const EXAMPLE_BASE_URL: &str = "http://192.168.0.10:11434";

pub const MODELS_PATH: &str = "/v1/models";
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
pub const EXAMPLE_TIMEOUT_MS: u32 = 600_000;
pub const AUTH_USES_BEARER: bool = true;

pub fn join_url(base: &str, path: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if path.starts_with('/') {
        format!("{b}{path}")
    } else {
        format!("{b}/{path}")
    }
}

/// Strip trailing `/` and a trailing `/v1` path segment (explicit, not a guess).
pub fn normalize_origin(raw: &str) -> Result<String, String> {
    let mut s = raw.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Err("endpointBaseUrl is empty".into());
    }
    if s.ends_with("/v1") {
        s.truncate(s.len() - 3);
        s = s.trim_end_matches('/').to_string();
    }
    if s.is_empty() {
        return Err("endpointBaseUrl is empty after stripping /v1".into());
    }
    reject_unspecified_host(&s)?;
    Ok(s)
}

pub fn reject_unspecified_host(origin: &str) -> Result<(), String> {
    let lower = origin.to_ascii_lowercase();
    if lower.contains("0.0.0.0") || lower.contains("[::]") || host_is_bare_unspecified(&lower) {
        return Err("origin must not use 0.0.0.0 or unspecified IPv6".into());
    }
    Ok(())
}

fn host_is_bare_unspecified(lower: &str) -> bool {
    // `http://[::]:11434` already caught; also `http://::1` is loopback (ok).
    // Bare `http://::/v1` is unspecified.
    if let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        let host = rest.split(['/', ':', '?']).next().unwrap_or("");
        return host == "::";
    }
    false
}
