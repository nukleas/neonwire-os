//! OCINT client — public civic intel API at https://ocint.app
//!
//! Uses the unauthenticated browser endpoints (same as the dashboard):
//!   GET /api/stats
//!   GET /api/items?limit=&category=
//!
//! v1 (`/api/v1/*`) needs `Authorization: Bearer ocint_<key>` — optional later
//! via `/mnt/sd/linux-lab/ocint.key`.

use std::time::{Duration, Instant};

use serde::Deserialize;

const DEFAULT_BASE: &str = "https://ocint.app";
const CONFIG_BASE: &str = "/mnt/sd/linux-lab/ocint.url";
const CONFIG_KEY: &str = "/mnt/sd/linux-lab/ocint.key";

/// Category chip set shown in the INTEL feed (mirrors ocint IntelFeed priorities).
pub const FILTERS: &[(&str, &str)] = &[
    ("all", "ALL"),
    ("crime", "CRM"),
    ("fire", "FIR"),
    ("traffic", "TRF"),
    ("power", "PWR"),
    ("weather", "WTH"),
    ("transit", "TRN"),
    ("earthquake", "EQK"),
    ("flood", "FLD"),
    ("airport", "APT"),
    ("news", "NWS"),
    ("government", "GOV"),
    ("event", "EVT"),
    ("health", "HLT"),
    ("air_quality", "AQI"),
    ("coastal", "CST"),
    ("permit", "PRM"),
];

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stats {
    #[serde(default)]
    pub total_today: i64,
    #[serde(default)]
    pub total_week: i64,
    #[serde(default)]
    pub by_category: std::collections::HashMap<String, i64>,
    #[serde(default)]
    pub active_fires: i64,
    #[serde(default)]
    pub active_outages: i64,
    pub current_aqi: Option<i64>,
    pub weather_summary: Option<String>,
    pub port_wind: Option<f64>,
    pub port_wave: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub title: String,
    pub summary: Option<String>,
    pub body: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    #[serde(default, alias = "publishedAt", alias = "published_at")]
    pub published_at: String,
    #[serde(default)]
    pub significance: i32,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub stats: Stats,
    pub items: Vec<Item>,
    pub fetched_at: Instant,
    pub base: String,
}

#[derive(Debug)]
pub enum FetchError {
    Network(String),
    Parse(String),
    Http(u16, String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Network(s) => write!(f, "net: {s}"),
            FetchError::Parse(s) => write!(f, "json: {s}"),
            FetchError::Http(c, s) => write!(f, "http {c}: {s}"),
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

fn api_key() -> Option<String> {
    std::fs::read_to_string(CONFIG_KEY)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("ocint_"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(15))
        .user_agent("neonwire-ocint/0.1 (dl7006; armv7)")
        .build()
}

fn get_json(url: &str) -> Result<String, FetchError> {
    let agent = agent();
    let mut req = agent.get(url);
    if let Some(k) = api_key() {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    match req.call() {
        Ok(resp) => resp.into_string().map_err(|e| FetchError::Network(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(FetchError::Http(code, body.chars().take(80).collect()))
        }
        Err(e) => Err(FetchError::Network(e.to_string())),
    }
}

/// Fetch stats + feed. `category` = None or "all" means balanced feed.
pub fn fetch(category: Option<&str>, limit: usize) -> Result<Snapshot, FetchError> {
    let base = base_url();
    let stats_url = format!("{base}/api/stats");
    let mut items_url = format!("{base}/api/items?limit={limit}");
    if let Some(c) = category {
        if c != "all" {
            items_url.push_str(&format!("&category={c}"));
        }
    }

    let stats_raw = get_json(&stats_url)?;
    let stats: Stats =
        serde_json::from_str(&stats_raw).map_err(|e| FetchError::Parse(e.to_string()))?;

    let items_raw = get_json(&items_url)?;
    let items: Vec<Item> =
        serde_json::from_str(&items_raw).map_err(|e| FetchError::Parse(e.to_string()))?;

    Ok(Snapshot { stats, items, fetched_at: Instant::now(), base })
}

/// Map ocint category → neon theme color (from CATEGORY_CONFIG hues).
pub fn cat_color(cat: &str) -> u32 {
    use neon_gfx::theme::*;
    match cat {
        "crime" | "health" => RED,
        "fire" | "earthquake" => AMBER,
        "traffic" | "permit" | "news" => GOLD,
        "power" => RED,
        "weather" | "flood" | "coastal" => BLUE,
        "transit" | "air_quality" => GREEN,
        "airport" | "event" => MAGENTA,
        "government" => AMBER,
        _ => TEXT2,
    }
}

pub fn cat_label(cat: &str) -> &str {
    FILTERS
        .iter()
        .find(|(k, _)| *k == cat)
        .map(|(_, l)| *l)
        .unwrap_or("???")
}

/// ASCII-safe for the 32..126 glyph atlas.
pub fn ascii(s: &str) -> String {
    s.chars()
        .map(|ch| {
            if ch.is_ascii_graphic() || ch == ' ' {
                ch
            } else if ch == '\u{2014}' || ch == '\u{2013}' {
                '-' // em/en dash
            } else if ch == '\u{00b0}' {
                ' ' // degree
            } else {
                '?'
            }
        })
        .collect()
}

/// Relative age from an RFC3339-ish timestamp, for the feed row.
pub fn age_label(published: &str) -> String {
    // parse loosely: take first 19 chars "YYYY-MM-DDTHH:MM:SS"
    let Some(head) = published.get(..19) else {
        return "??".into();
    };
    // convert to unix via libc-less manual parse
    let parts: Vec<&str> = head.split(&['T', '-', ':', ' '][..]).collect();
    if parts.len() < 6 {
        return "??".into();
    }
    let y: i64 = parts[0].parse().unwrap_or(0);
    let mo: i64 = parts[1].parse().unwrap_or(1);
    let d: i64 = parts[2].parse().unwrap_or(1);
    let h: i64 = parts[3].parse().unwrap_or(0);
    let mi: i64 = parts[4].parse().unwrap_or(0);
    let s: i64 = parts[5].parse().unwrap_or(0);
    let days = days_from_civil(y, mo, d);
    let pub_unix = days * 86400 + h * 3600 + mi * 60 + s;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = (now - pub_unix).max(0);
    if age < 60 {
        format!("{age}s")
    } else if age < 3600 {
        format!("{}m", age / 60)
    } else if age < 86400 {
        format!("{}h", age / 3600)
    } else {
        format!("{}d", age / 86400)
    }
}

// Howard Hinnant days_from_civil
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
