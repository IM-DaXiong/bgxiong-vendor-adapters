//! Schema-generated native host ports shared by reference and production hosts.
mod generated;
pub use generated::*;
pub use bgx_adapter_v2_sdk as wire;

pub fn decimal_u64(value: &str) -> Result<u64, &'static str> {
    let parsed = value.parse::<u64>().map_err(|_| "decimal value is outside u64")?;
    if parsed.to_string() != value { return Err("decimal value is not canonical"); }
    Ok(parsed)
}

pub fn resource_policy() -> &'static serde_json::Value {
    static POLICY: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    POLICY.get_or_init(|| serde_json::from_str(include_str!("../../../protocol/schema-policy.json")).expect("published policy"))
}

pub fn read_frame<R: std::io::Read>(reader: &mut R) -> std::io::Result<Option<bgx_adapter_v2_sdk::WorkerEnvelopeV2>> {
    let mut header = [0u8; core::mem::size_of::<u32>()];
    let mut offset = 0;
    while offset < header.len() {
        match reader.read(&mut header[offset..]) {
            Ok(0) if offset == 0 => return Ok(None),
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof,"truncated worker frame header")),
            Ok(count) => offset += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length as u64 > resource_policy()["limits"]["maxFrameBytes"].as_u64().unwrap() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"worker frame exceeds public resource budget"));
    }
    let mut payload = vec![0u8;length];reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map(Some).map_err(|_|std::io::Error::new(std::io::ErrorKind::InvalidData,"invalid worker frame contract"))
}

pub fn write_frame<W: std::io::Write>(writer: &mut W, frame: &bgx_adapter_v2_sdk::WorkerEnvelopeV2) -> std::io::Result<()> {
    let payload = serde_json::to_vec(frame).map_err(|_|std::io::Error::new(std::io::ErrorKind::InvalidData,"invalid worker frame"))?;
    let length = u32::try_from(payload.len()).map_err(|_|std::io::Error::new(std::io::ErrorKind::InvalidData,"worker frame length overflow"))?;
    if payload.len() as u64 > resource_policy()["limits"]["maxFrameBytes"].as_u64().unwrap() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"worker frame exceeds public resource budget"));
    }
    writer.write_all(&length.to_le_bytes())?;writer.write_all(&payload)?;writer.flush()
}
