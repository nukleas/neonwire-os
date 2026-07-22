//! ZeroClaw gateway client — talks to the on-device agent runtime.
//!
//! ZeroClaw (MIT/Apache-2, zeroclaw-labs/zeroclaw) runs as a SEPARATE daemon on
//! the tablet and binds an HTTP gateway on 127.0.0.1:42617. We only speak to it
//! over the wire, so nothing of it is linked into this binary.
//!
//! Contract verified against upstream `crates/zeroclaw-gateway/src/lib.rs`
//! (route table ~1576, handler ~2519, `WebhookBody` ~2503):
//!
//! ```text
//! POST {base}/webhook[?agent=<alias>]
//!   Authorization: Bearer <token>          # pairing token (POST /pair)
//!   Content-Type: application/json
//!   {"message": "..."}
//! -> 200 {"response": "...", "model": "..."}
//! -> 401/429/5xx {"error": "..."}
//! ```
//!
//! Config lives on the SD (never in the repo), mirroring the ocint.rs pattern:
//!   /mnt/sd/linux-lab/zeroclaw/{base,token,agent}

use std::time::Duration;

const CONFIG_BASE: &str = "/mnt/sd/linux-lab/zeroclaw/base";
const CONFIG_TOKEN: &str = "/mnt/sd/linux-lab/zeroclaw/token";
const CONFIG_AGENT: &str = "/mnt/sd/linux-lab/zeroclaw/agent";
const DEFAULT_BASE: &str = "http://127.0.0.1:42617";

pub struct Reply {
    pub text: String,
    pub model: Option<String>,
}

pub enum AskError {
    NoToken,
    Network(String),
    Http(u16, String),
    Parse(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::NoToken => write!(f, "no gateway token — pair zeroclaw first"),
            AskError::Network(s) => write!(f, "net: {s}"),
            AskError::Http(c, s) => write!(f, "http {c}: {s}"),
            AskError::Parse(s) => write!(f, "json: {s}"),
        }
    }
}

fn base_url() -> String {
    std::fs::read_to_string(CONFIG_BASE)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.into())
}

fn token() -> Option<String> {
    std::fs::read_to_string(CONFIG_TOKEN)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Optional agent alias; when absent the gateway picks the default agent.
fn agent_alias() -> Option<String> {
    std::fs::read_to_string(CONFIG_AGENT)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read timeout must exceed gateway.request_timeout_secs (180s on device).
/// Tool-using turns easily pass 30s; the gateway used to 408 at that floor.
fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(200))
        .user_agent("neonwire-zeroclaw/0.1 (dl7006; armv7)")
        .build()
}

/// Is the gateway up? Cheap unauthenticated liveness probe (`GET /health`).
pub fn health() -> bool {
    let url = format!("{}/health", base_url());
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(3))
        .build()
        .get(&url)
        .call()
        .is_ok()
}

/// Send one prompt to the agent, block until it replies. Runs on a worker
/// thread (see apps/assistant.rs) — never call this from the UI thread.
pub fn ask(message: &str) -> Result<Reply, AskError> {
    let Some(tok) = token() else {
        return Err(AskError::NoToken);
    };
    let mut url = format!("{}/webhook", base_url());
    if let Some(a) = agent_alias() {
        url.push_str(&format!("?agent={a}"));
    }

    let req = http()
        .post(&url)
        .set("Authorization", &format!("Bearer {tok}"))
        .set("Content-Type", "application/json");

    let body = match req.send_json(ureq::json!({ "message": message })) {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| AskError::Network(e.to_string()))?,
        Err(ureq::Error::Status(code, resp)) => {
            // gateway errors come back as {"error": "..."}
            let raw = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_else(|| raw.chars().take(80).collect());
            return Err(AskError::Http(code, msg));
        }
        Err(e) => return Err(AskError::Network(e.to_string())),
    };

    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AskError::Parse(e.to_string()))?;
    let text = v
        .get("response")
        .and_then(|r| r.as_str())
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return Err(AskError::Parse("empty response field".into()));
    }
    let model = v.get("model").and_then(|m| m.as_str()).map(String::from);
    Ok(Reply { text, model })
}
