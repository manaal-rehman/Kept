//! Related-document lookup over synced document metadata (Drive, OneDrive/SharePoint).

use crate::db::opt_ts;
use crate::models::Document;
use crate::text;
use anyhow::Result;
use rusqlite::Connection;

/// Documents whose names share meaningful words with `query` (best first).
pub fn related(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Document>> {
    let q = text::keywords(query);
    if q.is_empty() {
        return Ok(vec![]);
    }
    let mut st = conn.prepare("SELECT id, provider, name, mime_type, url, modified_at, modified_by FROM documents ORDER BY modified_at DESC LIMIT 2000")?;
    let rows = st.query_map([], |r| {
        Ok(Document { id: r.get(0)?, provider: r.get(1)?, name: r.get(2)?, mime_type: r.get(3)?, url: r.get(4)?, modified_at: opt_ts(r.get(5)?), modified_by: r.get(6)? })
    })?;
    let mut scored: Vec<(f64, Document)> = vec![];
    for d in rows {
        let d = d?;
        // Strip the file extension before comparing.
        let stem = d.name.rsplit_once('.').map(|(a, b)| if b.len() <= 5 { a } else { d.name.as_str() }).unwrap_or(&d.name).replace(['_', '-'], " ");
        let k = text::keywords(&stem);
        let shared: Vec<_> = q.intersection(&k).collect();
        // At least one substantial word must match; short tokens like "q4" only add to it.
        if !shared.iter().any(|w| w.len() >= 4) {
            continue;
        }
        let score = shared.len() as f64 / k.len().max(1) as f64 + shared.len() as f64 * 0.25;
        if shared.len() >= 2 || score >= 0.75 {
            scored.push((score, d));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored.into_iter().take(limit).map(|x| x.1).collect())
}
