//! YAML front-matter extractor.
//!
//! Finds a leading `---` … `---` block at the top of the markdown source,
//! parses it as YAML into [`crate::ir::FrontMatter`], and returns the body
//! with the block removed. Also resolves dynamic values (`date: auto`,
//! `date: today`, `date: now` → today's ISO date) so the user doesn't have
//! to maintain a date field by hand.
//!
//! If no front-matter is present, returns a default `FrontMatter` and the
//! original input untouched.

use crate::ir::FrontMatter;
use serde_yaml::Value;
use std::collections::BTreeMap;

/// Split markdown source into `(front_matter, body)`.
///
/// The leading UTF-8 BOM, if any, is stripped first. If the first
/// non-blank line is `---`, everything up to the next `---` is parsed as
/// YAML; on parse failure we silently return the default `FrontMatter` and
/// the original body — better to render a deck with a missing title than
/// to fail outright on a malformed YAML block.
pub fn extract(input: &str) -> (FrontMatter, String) {
    let input = input.trim_start_matches('\u{FEFF}');
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() || lines[0].trim() != "---" {
        return (FrontMatter::default(), input.to_string());
    }
    for i in 1..lines.len() {
        if lines[i].trim() == "---" {
            let yaml: String = lines[1..i].join("\n");
            let body: String = lines[i + 1..].join("\n");
            let mut front = match serde_yaml::from_str::<FrontMatter>(&yaml) {
                Ok(f) => f,
                Err(e) => {
                    // serde_yaml errors carry a `Location` with line/col
                    // numbers relative to the YAML chunk. We add 1 to the
                    // line so it counts against the original markdown
                    // source (line 1 of the YAML is line 2 of the file
                    // because of the opening `---`).
                    let loc = e.location();
                    if let Some(l) = loc {
                        eprintln!(
                            "md2any: warning — front-matter parse error at line {}, column {}: {} (using defaults)",
                            l.line() + 1,
                            l.column(),
                            e,
                        );
                    } else {
                        eprintln!(
                            "md2any: warning — front-matter parse error: {} (using defaults)",
                            e,
                        );
                    }
                    FrontMatter::default()
                }
            };
            resolve_math_macros(&mut front, &yaml);
            resolve_dynamic(&mut front);
            return (front, body);
        }
    }
    (FrontMatter::default(), input.to_string())
}

fn resolve_math_macros(front: &mut FrontMatter, yaml: &str) {
    if front.math_macros.is_some() {
        return;
    }
    let Ok(Value::Mapping(map)) = serde_yaml::from_str::<Value>(yaml) else {
        return;
    };
    let Some(value) = map
        .get(Value::String("math_macros".into()))
        .or_else(|| map.get(Value::String("math-macros".into())))
    else {
        return;
    };
    let Value::Mapping(macros) = value else {
        return;
    };
    let mut parsed = BTreeMap::new();
    for (key, value) in macros {
        let (Value::String(key), Value::String(value)) = (key, value) else {
            continue;
        };
        if !key.is_empty() {
            parsed.insert(key.clone(), value.clone());
        }
    }
    if !parsed.is_empty() {
        front.math_macros = Some(parsed);
    }
}

fn resolve_dynamic(front: &mut FrontMatter) {
    if let Some(d) = &front.date {
        let lower = d.trim().to_ascii_lowercase();
        if lower == "auto" || lower == "today" || lower == "now" {
            front.date = Some(today_iso());
        }
    }
}

/// Today's date in YYYY-MM-DD using the system clock (UTC).
fn today_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m, d)
}
