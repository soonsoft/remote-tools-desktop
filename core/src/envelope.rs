use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolName {
    #[serde(rename = "client__bash")] Bash,
    #[serde(rename = "client__read")] Read,
    #[serde(rename = "client__write")] Write,
    #[serde(rename = "client__edit")] Edit,
    #[serde(rename = "client__grep")] Grep,
    #[serde(rename = "client__glob")] Glob,
}

impl ToolName {
    pub fn wire(&self) -> &'static str {
        match self {
            Self::Bash => "client__bash", Self::Read => "client__read",
            Self::Write => "client__write", Self::Edit => "client__edit",
            Self::Grep => "client__grep", Self::Glob => "client__glob",
        }
    }
    pub fn from_wire(s: &str) -> Option<Self> {
        ["client__bash","client__read","client__write","client__edit","client__grep","client__glob"]
            .into_iter().zip([Self::Bash,Self::Read,Self::Write,Self::Edit,Self::Grep,Self::Glob])
            .find(|(w, _)| *w == s).map(|(_, t)| t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "client_offline")] ClientOffline,
    #[serde(rename = "timeout")] Timeout,
    #[serde(rename = "denied_by_user")] DeniedByUser,
    #[serde(rename = "denied_by_rule")] DeniedByRule,
    #[serde(rename = "path_denied")] PathDenied,
    #[serde(rename = "bad_args")] BadArgs,
    #[serde(rename = "exec_error")] ExecError,
    #[serde(rename = "output_too_large")] OutputTooLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta { #[serde(rename = "requestedAt")] pub requested_at: i64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    pub id: String, pub tool: ToolName, pub args: Value, pub meta: Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError { pub code: ErrorCode, pub message: String }

#[derive(Debug, Clone)]
pub enum ToolResponse {
    Ok { id: String, result: Value },
    Err { id: String, error: ToolError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello { pub hostname: String, pub platform: String }

#[derive(Debug, Clone)]
pub enum Envelope {
    ToolRequest(ToolRequest),
    ToolResponse(ToolResponse),
    Hello(Hello),
    HelloAck,
    Ping,
    Pong,
}

pub fn new_request_id() -> String { ulid::Ulid::new().to_string() }

pub fn encode_envelope(e: &Envelope) -> String {
    let v = match e {
        Envelope::ToolRequest(r) => serde_json::json!({
            "v": 1, "type": "tool_request", "id": r.id, "tool": r.tool.wire(),
            "args": r.args, "meta": { "requestedAt": r.meta.requested_at } }),
        Envelope::ToolResponse(ToolResponse::Ok { id, result }) => serde_json::json!({
            "v": 1, "type": "tool_response", "id": id, "ok": true, "result": result }),
        Envelope::ToolResponse(ToolResponse::Err { id, error }) => serde_json::json!({
            "v": 1, "type": "tool_response", "id": id, "ok": false,
            "error": { "code": error.code, "message": error.message } }),
        Envelope::Hello(h) => serde_json::json!({
            "v": 1, "type": "hello", "hostname": h.hostname, "platform": h.platform }),
        Envelope::HelloAck => serde_json::json!({ "v": 1, "type": "hello_ack", "server": "remote-tools" }),
        Envelope::Ping => serde_json::json!({ "v": 1, "type": "ping" }),
        Envelope::Pong => serde_json::json!({ "v": 1, "type": "pong" }),
    };
    serde_json::to_string(&v).expect("envelope serializes")
}

/// 严格解析（镜像 Plan A parseEnvelope 的全部拒绝路径；serde 校验字段类型）。
pub fn parse_envelope(raw: &str) -> Option<Envelope> {
    let v: Value = serde_json::from_str(raw).ok()?;
    if v.get("v")?.as_i64()? != 1 { return None; }
    match v.get("type")?.as_str()? {
        "tool_request" => Some(Envelope::ToolRequest(serde_json::from_value(v).ok()?)),
        "tool_response" => {
            let id = v.get("id")?.as_str()?.to_string();
            match v.get("ok")?.as_bool()? {
                true => {
                    let result = v.get("result")?.clone();   // 缺 result → None（Plan A 修订）
                    Some(Envelope::ToolResponse(ToolResponse::Ok { id, result }))
                }
                false => Some(Envelope::ToolResponse(ToolResponse::Err {
                    id,
                    error: serde_json::from_value(v.get("error")?.clone()).ok()?,
                })),
            }
        }
        "hello" => Some(Envelope::Hello(serde_json::from_value(v).ok()?)),
        "hello_ack" => Some(Envelope::HelloAck),
        "ping" => Some(Envelope::Ping),
        "pong" => Some(Envelope::Pong),
        _ => None,
    }
}
