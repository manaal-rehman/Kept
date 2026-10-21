//! Document metadata from Google Drive and OneDrive / SharePoint.
//! Only names, links and modification info are read — never file contents. Kept uses them
//! to show "related documents" next to tasks and meetings (e.g. the report a task mentions).

use super::http::{base, get_json, request_json, Auth};
use super::{ConnectorError, SyncOutput};
use crate::models::IncomingDocument;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

pub const GDRIVE_SCOPES: &[&str] = &["openid", "email", "https://www.googleapis.com/auth/drive.metadata.readonly"];
pub const ONEDRIVE_SCOPES: &[&str] = &["offline_access", "User.Read", "Files.Read.All"];
const MAX_FILES: usize = 300;
const WINDOW_DAYS: i64 = 60;

fn ts(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

pub fn parse_drive_file(f: &Value) -> Option<IncomingDocument> {
    if f["trashed"].as_bool() == Some(true) {
        return None;
    }
    Some(IncomingDocument {
        external_id: f["id"].as_str()?.to_string(),
        name: f["name"].as_str()?.to_string(),
        mime_type: f["mimeType"].as_str().map(|s| s.to_string()),
        url: f["webViewLink"].as_str().map(|s| s.to_string()),
        modified_at: ts(&f["modifiedTime"]),
        modified_by: f["lastModifyingUser"]["displayName"].as_str().map(|s| s.to_string()),
        owner: f["owners"][0]["displayName"].as_str().map(|s| s.to_string()),
    })
}

pub async fn sync_gdrive(http: &reqwest::Client, token: &str, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let since = (Utc::now() - Duration::days(WINDOW_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let q = urlencoding::encode(&format!("modifiedTime > '{since}' and trashed = false and mimeType != 'application/vnd.google-apps.folder'")).to_string();
    let fields = urlencoding::encode("nextPageToken,files(id,name,mimeType,modifiedTime,webViewLink,trashed,owners(displayName),lastModifyingUser(displayName))").to_string();
    let api = base("KEPT_TEST_GDRIVE_BASE", "https://www.googleapis.com/drive/v3");
    let mut page: Option<String> = None;
    loop {
        let mut url = format!("{api}/files?q={q}&fields={fields}&orderBy=modifiedTime%20desc&pageSize=100&includeItemsFromAllDrives=true&supportsAllDrives=true");
        if let Some(p) = &page {
            url.push_str(&format!("&pageToken={p}"));
        }
        let v = get_json(http, &url, token, &[]).await?;
        out.documents.extend(v["files"].as_array().cloned().unwrap_or_default().iter().filter_map(parse_drive_file));
        page = v["nextPageToken"].as_str().map(|s| s.to_string());
        if page.is_none() || out.documents.len() >= MAX_FILES {
            break;
        }
    }
    out.cursors.push(("documents".into(), Utc::now().to_rfc3339()));
    Ok(())
}

pub fn parse_drive_item(r: &Value) -> Option<IncomingDocument> {
    if r["folder"].is_object() {
        return None;
    }
    Some(IncomingDocument {
        external_id: r["id"].as_str()?.to_string(),
        name: r["name"].as_str()?.to_string(),
        mime_type: r["file"]["mimeType"].as_str().map(|s| s.to_string()),
        url: r["webUrl"].as_str().map(|s| s.to_string()),
        modified_at: ts(&r["lastModifiedDateTime"]),
        modified_by: r["lastModifiedBy"]["user"]["displayName"].as_str().map(|s| s.to_string()),
        owner: r["createdBy"]["user"]["displayName"].as_str().map(|s| s.to_string()),
    })
}

/// OneDrive + SharePoint files you can access, via the Microsoft Search API
/// (replacement for the deprecated /me/drive/recent and /sharedWithMe endpoints).
pub async fn sync_onedrive(http: &reqwest::Client, token: &str, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let graph = base("KEPT_TEST_GRAPH_BASE", "https://graph.microsoft.com/v1.0");
    let since = (Utc::now() - Duration::days(WINDOW_DAYS)).format("%Y-%m-%d");
    let mut from = 0;
    while from < MAX_FILES {
        let body = json!({"requests": [{
            "entityTypes": ["driveItem"],
            "query": {"queryString": format!("LastModifiedTime>={since}")},
            "from": from, "size": 100,
            "fields": ["id", "name", "webUrl", "lastModifiedDateTime", "lastModifiedBy", "createdBy", "file", "folder"]
        }]});
        let v = request_json(http, reqwest::Method::POST, &format!("{graph}/search/query"), Auth::Bearer(token), Some(&body), &[]).await?;
        let container = &v["value"][0]["hitsContainers"][0];
        let hits = container["hits"].as_array().cloned().unwrap_or_default();
        out.documents.extend(hits.iter().filter_map(|h| parse_drive_item(&h["resource"])));
        if container["moreResultsAvailable"].as_bool() != Some(true) || hits.is_empty() {
            break;
        }
        from += 100;
    }
    out.cursors.push(("documents".into(), Utc::now().to_rfc3339()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drive_and_graph_files() {
        let f = json!({"id": "1", "name": "Q3 revenue report.docx", "mimeType": "application/vnd.openxmlformats", "modifiedTime": "2026-10-17T10:00:00Z", "webViewLink": "https://docs.google.com/x", "lastModifyingUser": {"displayName": "Sara"}});
        let d = parse_drive_file(&f).unwrap();
        assert_eq!(d.name, "Q3 revenue report.docx");
        assert_eq!(d.modified_by.as_deref(), Some("Sara"));
        let g = json!({"id": "A1", "name": "Board deck.pptx", "webUrl": "https://contoso.sharepoint.com/x", "lastModifiedDateTime": "2026-10-16T09:00:00Z", "file": {"mimeType": "application/pptx"}});
        assert_eq!(parse_drive_item(&g).unwrap().name, "Board deck.pptx");
        assert!(parse_drive_item(&json!({"id": "F", "name": "folder", "folder": {}})).is_none());
    }
}
