//! File imports: iCalendar (.ics), RFC 5322 emails (.eml), and Kept JSON bundles.

use crate::models::{Attendee, IncomingEvent, IncomingMessage};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

fn unfold(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in s.replace("\r\n", "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else {
            out.push(line.to_string());
        }
    }
    out
}

fn ics_unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

fn parse_ics_time(params: &str, value: &str, default_tz: Tz) -> Option<(DateTime<Utc>, bool)> {
    let value = value.trim();
    if params.contains("VALUE=DATE") || (value.len() == 8 && !value.contains('T')) {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?.and_utc(), true));
    }
    if let Some(v) = value.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
        return Some((n.and_utc(), false));
    }
    let tz: Tz = params.split(';').find_map(|p| p.strip_prefix("TZID=")).and_then(|t| t.trim_matches('"').parse().ok()).unwrap_or(default_tz);
    let n = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let d = tz.from_local_datetime(&n).earliest()?;
    Some((d.with_timezone(&Utc), false))
}

/// Parse VEVENTs. RRULE expansion is limited to simple WEEKLY/DAILY rules within 30 days.
pub fn parse_ics(content: &str, default_tz: Tz) -> Result<Vec<IncomingEvent>> {
    let mut out = vec![];
    let mut cur: Option<IncomingEvent> = None;
    let mut rrule: Option<String> = None;
    for line in unfold(content) {
        let Some((head, value)) = line.split_once(':') else { continue };
        let (name, params) = head.split_once(';').map(|(a, b)| (a, b)).unwrap_or((head, ""));
        match (name.to_ascii_uppercase().as_str(), cur.as_mut()) {
            ("BEGIN", None) if value.trim() == "VEVENT" => {
                cur = Some(IncomingEvent { status: "confirmed".into(), ..Default::default() });
                rrule = None;
            }
            ("END", Some(_)) if value.trim() == "VEVENT" => {
                let ev = cur.take().unwrap();
                if ev.external_id.is_empty() || ev.title.is_empty() {
                    continue;
                }
                let ev = IncomingEvent { end_at: if ev.end_at <= ev.start_at { ev.start_at + Duration::minutes(30) } else { ev.end_at }, ..ev };
                if let Some(r) = &rrule {
                    let step = if r.contains("FREQ=DAILY") { Some(Duration::days(1)) } else if r.contains("FREQ=WEEKLY") { Some(Duration::weeks(1)) } else { None };
                    if let Some(step) = step {
                        let horizon = Utc::now() + Duration::days(30);
                        let mut s = ev.start_at;
                        let len = ev.end_at - ev.start_at;
                        let mut i = 0;
                        while s < horizon && i < 60 {
                            if s > Utc::now() - Duration::days(2) {
                                out.push(IncomingEvent { external_id: format!("{}#{}", ev.external_id, s.timestamp()), start_at: s, end_at: s + len, ..ev.clone() });
                            }
                            s += step;
                            i += 1;
                        }
                        continue;
                    }
                }
                out.push(ev);
            }
            ("UID", Some(e)) => e.external_id = value.trim().to_string(),
            ("SUMMARY", Some(e)) => e.title = ics_unescape(value.trim()),
            ("DESCRIPTION", Some(e)) => e.description = Some(ics_unescape(value)),
            ("LOCATION", Some(e)) => e.location = Some(ics_unescape(value)).filter(|s| !s.is_empty()),
            ("URL", Some(e)) => e.url = Some(value.trim().to_string()),
            ("STATUS", Some(e)) => e.status = value.trim().to_lowercase(),
            ("RRULE", Some(_)) => rrule = Some(value.to_string()),
            ("DTSTART", Some(e)) => {
                if let Some((t, all)) = parse_ics_time(params, value, default_tz) {
                    e.start_at = t;
                    e.all_day = all;
                    e.timezone = params.split(';').find_map(|p| p.strip_prefix("TZID=")).map(|s| s.to_string());
                }
            }
            ("DTEND", Some(e)) => {
                if let Some((t, _)) = parse_ics_time(params, value, default_tz) {
                    e.end_at = t;
                }
            }
            ("ORGANIZER", Some(e)) => {
                e.organizer_email = value.trim().strip_prefix("mailto:").or(value.trim().strip_prefix("MAILTO:")).map(|s| s.to_lowercase());
                e.organizer_name = params.split(';').find_map(|p| p.strip_prefix("CN=")).map(|s| s.trim_matches('"').to_string());
            }
            ("ATTENDEE", Some(e)) => {
                let email = value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_lowercase();
                if email.contains('@') {
                    e.attendees.push(Attendee {
                        email,
                        name: params.split(';').find_map(|p| p.strip_prefix("CN=")).map(|s| s.trim_matches('"').to_string()),
                        response: params.split(';').find_map(|p| p.strip_prefix("PARTSTAT=")).map(|s| s.to_lowercase()),
                        optional: params.contains("ROLE=OPT-PARTICIPANT"),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn parse_eml(bytes: &[u8], file_name: &str) -> Result<IncomingMessage> {
    let msg = mail_parser::MessageParser::default().parse(bytes).ok_or_else(|| anyhow!("{file_name}: not a valid email"))?;
    let fmt = |a: Option<&mail_parser::Address>| -> Vec<String> {
        a.map(|a| {
            a.iter()
                .filter_map(|x| {
                    let e = x.address()?;
                    Some(match x.name() {
                        Some(n) => format!("{n} <{e}>"),
                        None => e.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let from = msg.from().and_then(|a| a.first());
    let body = msg.body_text(0).map(|b| b.to_string()).or_else(|| msg.body_html(0).map(|h| super::http::html_to_text(&h))).unwrap_or_default();
    let id = msg.message_id().map(|s| s.to_string()).unwrap_or_else(|| crate::text::sha256_hex(&String::from_utf8_lossy(bytes))[..24].to_string());
    let thread = msg
        .references()
        .as_text_list()
        .and_then(|l| l.first().map(|s| s.to_string()))
        .or_else(|| msg.in_reply_to().as_text().map(|s| s.to_string()))
        .unwrap_or_else(|| id.clone());
    let sent = msg.date().map(|d| DateTime::<Utc>::from_timestamp(d.to_timestamp(), 0).unwrap_or_else(Utc::now)).unwrap_or_else(Utc::now);
    Ok(IncomingMessage {
        external_id: id,
        kind: "email".into(),
        thread_key: Some(format!("eml:{thread}")),
        sender_name: from.and_then(|f| f.name()).map(|s| s.to_string()),
        sender_email: from.and_then(|f| f.address()).map(|s| s.to_lowercase()),
        to: fmt(msg.to()),
        cc: fmt(msg.cc()),
        subject: msg.subject().map(|s| s.to_string()),
        body_text: body,
        url: None,
        sent_at: sent,
    })
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Bundle {
    pub messages: Vec<IncomingMessage>,
    pub events: Vec<IncomingEvent>,
}

pub fn parse_bundle(json: &str) -> Result<Bundle> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ics_with_tz_and_attendees() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:abc\r\nSUMMARY:Project Review\r\nDTSTART;TZID=Asia/Karachi:20261019T100000\r\nDTEND;TZID=Asia/Karachi:20261019T110000\r\nORGANIZER;CN=Lead:mailto:lead@corp.com\r\nATTENDEE;CN=\"Sara Khan\";PARTSTAT=ACCEPTED:mailto:Sara@corp.com\r\nDESCRIPTION:Agenda:\\n1. Budget\\, timeline\r\n continued\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let evs = parse_ics(ics, chrono_tz::UTC).unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].start_at.to_rfc3339(), "2026-10-19T05:00:00+00:00");
        assert_eq!(evs[0].attendees[0].email, "sara@corp.com");
        assert!(evs[0].description.as_ref().unwrap().contains("Budget, timelinecontinued"));
    }

    #[test]
    fn parses_eml() {
        let eml = b"From: Sara Khan <sara@acme.com>\r\nTo: me@corp.com\r\nSubject: Re: analysis\r\nMessage-ID: <m1@acme.com>\r\nIn-Reply-To: <m0@acme.com>\r\nDate: Tue, 13 Oct 2026 10:00:00 +0500\r\n\r\nCould you send me the updated analysis before Thursday?\r\n";
        let m = parse_eml(eml, "a.eml").unwrap();
        assert_eq!(m.sender_email.as_deref(), Some("sara@acme.com"));
        assert_eq!(m.thread_key.as_deref(), Some("eml:m0@acme.com"));
        assert_eq!(m.sent_at.to_rfc3339(), "2026-10-13T05:00:00+00:00");
    }
}
