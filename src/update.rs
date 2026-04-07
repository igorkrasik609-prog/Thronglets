//! Update checker — background npm registry check with 1-hour cache.
//!
//! Never blocks startup. Never panics. Fire-and-forget.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PACKAGE_NAME: &str = "thronglets";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CHECK_INTERVAL_SECS: u64 = 3600; // 1 hour
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".thronglets")
        .join("update-check.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..3 {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        match va.cmp(&vb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Read cached check result. Returns (last_check_secs, latest_version).
fn read_cache() -> Option<(u64, String)> {
    let data = fs::read_to_string(cache_path()).ok()?;
    let obj: serde_json::Value = serde_json::from_str(&data).ok()?;
    let ts = obj.get("lastCheck")?.as_u64()?;
    let ver = obj.get("latestVersion")?.as_str()?.to_string();
    Some((ts, ver))
}

fn write_cache(latest: &str) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let obj = serde_json::json!({
        "lastCheck": now_secs(),
        "latestVersion": latest,
    });
    let _ = fs::write(path, serde_json::to_string(&obj).unwrap_or_default());
}

fn fetch_latest_version() -> Option<String> {
    let url = format!("https://registry.npmjs.org/{PACKAGE_NAME}/latest");
    let resp = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .ok()?
        .get(&url)
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let obj: serde_json::Value = resp.json().ok()?;
    obj.get("version")?.as_str().map(String::from)
}

/// Synchronous check — runs in a background thread.
fn check_sync() {
    // Check cache first
    if let Some((ts, ver)) = read_cache()
        && now_secs() - ts < CHECK_INTERVAL_SECS
    {
        if compare_semver(CURRENT_VERSION, &ver) == std::cmp::Ordering::Less {
            eprintln!(
                "[thronglets] v{ver} available (current: v{CURRENT_VERSION}). \
                 Run: npx -y thronglets@latest start"
            );
        }
        return;
    }

    let Some(latest) = fetch_latest_version() else {
        return;
    };
    write_cache(&latest);

    if compare_semver(CURRENT_VERSION, &latest) == std::cmp::Ordering::Less {
        eprintln!(
            "[thronglets] v{latest} available (current: v{CURRENT_VERSION}). \
             Run: npx -y thronglets@latest start"
        );
    }
}

/// Fire-and-forget update check. Spawns a background thread.
/// Never blocks, never panics.
pub fn check_for_update() {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(|| {
            if let Err(e) = std::panic::catch_unwind(check_sync) {
                tracing::debug!("update check panicked: {e:?}");
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_semver("0.7.10", "0.7.10"), Equal);
        assert_eq!(compare_semver("0.7.10", "0.7.11"), Less);
        assert_eq!(compare_semver("0.7.11", "0.7.10"), Greater);
        assert_eq!(compare_semver("1.0.0", "0.99.99"), Greater);
        assert_eq!(compare_semver("0.7.10", "0.8.0"), Less);
    }
}
