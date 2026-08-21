//! Home Assistant REST client (long-lived access token).
//!
//! Config on the tablet (SD lab dir, never commit secrets):
//!   /mnt/sd/linux-lab/hass.url       e.g. http://homeassistant.local:8123
//!   /mnt/sd/linux-lab/hass.token     long-lived access token
//!   /mnt/sd/linux-lab/hass.entities  optional allowlist (one entity_id per line)
//!
//! API (same as home-agent tools):
//!   GET  /api/          ping
//!   GET  /api/states    all entity states
//!   POST /api/services/{domain}/{service}  with {"entity_id": ...}

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

const CONFIG_URL: &str = "/mnt/sd/linux-lab/hass.url";
const CONFIG_TOKEN: &str = "/mnt/sd/linux-lab/hass.token";
const CONFIG_ENTITIES: &str = "/mnt/sd/linux-lab/hass.entities";

/// Domains we surface in the HOUSE UI.
pub const FILTERS: &[(&str, &str)] = &[
    ("all", "ALL"),
    ("light", "LIGHT"),
    ("switch", "SW"),
    ("climate", "CLIM"),
    ("fan", "FAN"),
    ("script", "SCRIPT"),
    ("sensor", "SENS"),
    ("binary_sensor", "BIN"),
    ("media_player", "MEDIA"),
];

#[derive(Debug, Clone)]
pub struct Entity {
    pub entity_id: String,
    pub domain: String,
    pub state: String,
    pub name: String,
    /// Extra one-line readout (temp, unit, brightness, …).
    pub detail: String,
    pub toggleable: bool,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub entities: Vec<Entity>,
    pub weather: Option<String>,
    pub climate: Option<String>,
    pub lights_on: usize,
    pub switches_on: usize,
    pub total: usize,
    pub fetched_at: Instant,
    pub base: String,
}

#[derive(Debug)]
pub enum HassError {
    Config(String),
    Network(String),
    Http(u16, String),
    Parse(String),
}

impl std::fmt::Display for HassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HassError::Config(s) => write!(f, "cfg: {s}"),
            HassError::Network(s) => write!(f, "net: {s}"),
            HassError::Http(c, s) => write!(f, "http {c}: {s}"),
            HassError::Parse(s) => write!(f, "json: {s}"),
        }
    }
}

fn base_url() -> Result<String, HassError> {
    std::fs::read_to_string(CONFIG_URL)
        .map_err(|_| {
            HassError::Config(
                "missing hass.url (e.g. http://homeassistant.local:8123)".into(),
            )
        })
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .and_then(|s| {
            if s.is_empty() {
                Err(HassError::Config("empty hass.url".into()))
            } else {
                Ok(s)
            }
        })
}

fn token() -> Result<String, HassError> {
    std::fs::read_to_string(CONFIG_TOKEN)
        .map_err(|_| {
            HassError::Config("missing hass.token (LLAT from HA Profile > Security)".into())
        })
        .map(|s| s.trim().to_string())
        .and_then(|s| {
            if s.is_empty() {
                Err(HassError::Config("empty hass.token".into()))
            } else {
                Ok(s)
            }
        })
}

fn allowlist() -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(CONFIG_ENTITIES).ok()?;
    let list: Vec<String> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect();
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout_read(Duration::from_secs(20))
        .user_agent("neonwire-hass/0.1 (dl7006; armv7)")
        .build()
}

fn get_json(path: &str) -> Result<String, HassError> {
    let base = base_url()?;
    let tok = token()?;
    let url = format!("{base}{path}");
    match agent().get(&url).set("Authorization", &format!("Bearer {tok}")).call() {
        Ok(resp) => resp.into_string().map_err(|e| HassError::Network(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(HassError::Http(code, body.chars().take(80).collect()))
        }
        Err(e) => Err(HassError::Network(e.to_string())),
    }
}

fn post_json(path: &str, body: &Value) -> Result<(), HassError> {
    let base = base_url()?;
    let tok = token()?;
    let url = format!("{base}{path}");
    match agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {tok}"))
        .set("Content-Type", "application/json")
        .send_json(body.clone())
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(HassError::Http(code, body.chars().take(80).collect()))
        }
        Err(e) => Err(HassError::Network(e.to_string())),
    }
}

/// Ping HA (`GET /api/`).
pub fn ping() -> Result<String, HassError> {
    let raw = get_json("/api/")?;
    #[derive(Deserialize)]
    struct Msg {
        message: Option<String>,
    }
    let m: Msg = serde_json::from_str(&raw).map_err(|e| HassError::Parse(e.to_string()))?;
    Ok(m.message.unwrap_or_else(|| "ok".into()))
}

/// Pull states and project into a HOUSE-friendly snapshot.
pub fn fetch() -> Result<Snapshot, HassError> {
    let base = base_url()?;
    let raw = get_json("/api/states")?;
    let rows: Vec<Value> =
        serde_json::from_str(&raw).map_err(|e| HassError::Parse(e.to_string()))?;

    let allow = allowlist();
    let mut entities = Vec::new();
    let mut weather = None;
    let mut climate = None;
    let mut lights_on = 0usize;
    let mut switches_on = 0usize;

    for row in &rows {
        let entity_id = row.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
        if entity_id.is_empty() {
            continue;
        }
        let domain = entity_id.split('.').next().unwrap_or("");
        let state = row.get("state").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        let attrs = row.get("attributes").cloned().unwrap_or(Value::Null);
        let name = attrs
            .get("friendly_name")
            .and_then(|v| v.as_str())
            .unwrap_or(entity_id)
            .to_string();

        // weather / climate banners
        if domain == "weather" && weather.is_none() && state != "unavailable" {
            let temp = attrs.get("temperature").and_then(|v| as_f64(v));
            let unit = attrs
                .get("temperature_unit")
                .and_then(|v| v.as_str())
                .unwrap_or("F");
            let unit = ascii(unit).replace('?', "F"); // ° often becomes ?
            weather = Some(match temp {
                Some(t) => format!("{}  {:.0}{}", ascii(&state), t, unit),
                None => ascii(&state),
            });
        }
        if domain == "climate" && climate.is_none() && state != "unavailable" {
            let cur = attrs.get("current_temperature").and_then(|v| as_f64(v));
            let tgt = attrs.get("temperature").and_then(|v| as_f64(v));
            climate = Some(match (cur, tgt) {
                (Some(c), Some(t)) => format!("{}  {:.0}/{:.0}F", ascii(&name), c, t),
                (Some(c), None) => format!("{}  {:.0}F", ascii(&name), c),
                _ => format!("{}  {}", ascii(&name), ascii(&state)),
            });
        }

        // domain filter for the entity grid
        let interesting = matches!(
            domain,
            "light"
                | "switch"
                | "climate"
                | "fan"
                | "script"
                | "sensor"
                | "binary_sensor"
                | "media_player"
                | "cover"
                | "lock"
                | "input_boolean"
        );
        if !interesting {
            continue;
        }
        if let Some(list) = &allow {
            if !list.iter().any(|id| id == entity_id) {
                continue;
            }
        }
        // sensors are noisy — keep only those with a unit or temperature-ish name
        if domain == "sensor" {
            let unit = attrs.get("unit_of_measurement").and_then(|v| v.as_str());
            let keep = unit.is_some()
                || name.to_lowercase().contains("temp")
                || name.to_lowercase().contains("humid")
                || name.to_lowercase().contains("battery")
                || name.to_lowercase().contains("power")
                || name.to_lowercase().contains("energy");
            if !keep {
                continue;
            }
        }
        // drop unavailable noise unless allowlisted
        if matches!(state.as_str(), "unavailable" | "unknown") && allow.is_none() {
            continue;
        }

        if domain == "light" && state == "on" {
            lights_on += 1;
        }
        if domain == "switch" && state == "on" {
            switches_on += 1;
        }

        let detail = entity_detail(domain, &state, &attrs);
        let toggleable = matches!(domain, "light" | "switch" | "fan" | "input_boolean" | "script");

        entities.push(Entity {
            entity_id: entity_id.to_string(),
            domain: domain.to_string(),
            state,
            name: ascii(&name),
            detail,
            toggleable,
        });
    }

    // lights/switches first, then climate, then the rest — alphabetical within
    entities.sort_by(|a, b| {
        domain_rank(&a.domain)
            .cmp(&domain_rank(&b.domain))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let total = entities.len();
    Ok(Snapshot {
        entities,
        weather,
        climate,
        lights_on,
        switches_on,
        total,
        fetched_at: Instant::now(),
        base,
    })
}

/// Toggle a toggleable entity (or fire a script).
pub fn toggle(entity_id: &str) -> Result<(), HassError> {
    let domain = entity_id.split('.').next().unwrap_or("");
    let (svc_domain, service) = match domain {
        "light" | "switch" | "fan" | "input_boolean" => (domain, "toggle"),
        "script" => ("script", "turn_on"),
        "lock" => {
            // lock.toggle not always present — caller should pass state
            return Err(HassError::Config("lock needs unlock/lock, not toggle".into()));
        }
        _ => return Err(HassError::Config(format!("not toggleable: {domain}"))),
    };
    post_json(
        &format!("/api/services/{svc_domain}/{service}"),
        &json!({ "entity_id": entity_id }),
    )
}

/// Turn entity on/off explicitly (for optimistic UI after reading state).
pub fn turn(entity_id: &str, on: bool) -> Result<(), HassError> {
    let domain = entity_id.split('.').next().unwrap_or("");
    if !matches!(domain, "light" | "switch" | "fan" | "input_boolean") {
        return toggle(entity_id);
    }
    let service = if on { "turn_on" } else { "turn_off" };
    post_json(
        &format!("/api/services/{domain}/{service}"),
        &json!({ "entity_id": entity_id }),
    )
}

fn domain_rank(d: &str) -> u8 {
    match d {
        "light" => 0,
        "switch" => 1,
        "climate" => 2,
        "fan" => 3,
        "cover" | "lock" => 4,
        "script" => 5,
        "media_player" => 6,
        "binary_sensor" => 7,
        "sensor" => 8,
        _ => 9,
    }
}

fn entity_detail(domain: &str, state: &str, attrs: &Value) -> String {
    match domain {
        "climate" => {
            let cur = attrs.get("current_temperature").and_then(|v| as_f64(v));
            let tgt = attrs.get("temperature").and_then(|v| as_f64(v));
            match (cur, tgt) {
                (Some(c), Some(t)) => format!("{state}  {c:.0}/{t:.0}F"),
                (Some(c), None) => format!("{state}  {c:.0}F"),
                _ => state.to_string(),
            }
        }
        "sensor" => {
            let unit = attrs
                .get("unit_of_measurement")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if unit.is_empty() {
                ascii(state)
            } else {
                format!("{} {}", ascii(state), unit)
            }
        }
        "light" if state == "on" => {
            if let Some(b) = attrs.get("brightness").and_then(|v| v.as_u64()) {
                format!("on  {}%", b * 100 / 255)
            } else {
                "on".into()
            }
        }
        "media_player" => ascii(state),
        "binary_sensor" => ascii(state),
        _ => ascii(state),
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_str()?.parse().ok())
}

pub fn domain_color(domain: &str) -> u32 {
    use neon_gfx::theme::*;
    match domain {
        "light" => AMBER,
        "switch" => CYAN,
        "climate" => MAGENTA,
        "fan" => BLUE,
        "script" => PURPLE,
        "sensor" => TEXT2,
        "binary_sensor" => GREEN,
        "media_player" => GOLD,
        "cover" | "lock" => RED,
        _ => TEXT2,
    }
}

pub fn ascii(s: &str) -> String {
    s.chars()
        .map(|ch| {
            if ch.is_ascii_graphic() || ch == ' ' {
                ch
            } else if matches!(ch, '\u{2014}' | '\u{2013}') {
                '-'
            } else if ch == '\u{00b0}' {
                ' '
            } else if ch == '\u{2019}' || ch == '\u{2018}' {
                '\''
            } else {
                '?'
            }
        })
        .collect()
}
