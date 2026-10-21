//! Deterministic deadline interpretation.
//!
//! The AI layer may *propose* a deadline phrase, but the actual date is always
//! computed here, relative to when the message was sent and in the user's
//! timezone. This keeps date arithmetic predictable and testable.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeadlineGuess {
    pub due_at: DateTime<Utc>,
    /// The words we based this on, e.g. "before Thursday".
    pub phrase: String,
    /// explicit = a stated deadline ("by Friday"); inferred = a stated time of action
    /// ("I'll send it tomorrow") or a vague period ("next week").
    pub kind: String,
    /// True when the sentence ties the deadline to a meeting ("before our Monday meeting").
    pub before_meeting: bool,
    /// Weekday the meeting is on, if named.
    pub meeting_weekday: Option<Weekday>,
    /// Words that describe which meeting ("project review", "standup"), if any.
    #[serde(default)]
    pub meeting_hint: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkDay {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl Default for WorkDay {
    fn default() -> Self {
        Self { start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(17, 30, 0).unwrap() }
    }
}

static WEEKDAY: &str = r"(monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tues|tue|wed|thurs|thu|fri|sat|sun)";
static MONTHS: &str = r"(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec)";

static RE_MEETING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"\b(before|ahead of|prior to|for|by)\s+(?:our|the|my|your|this|next|tomorrow'?s|today'?s)?\s*(?:{WEEKDAY}(?:'s)?\s+)?((?:[a-z0-9:]+\s+){{0,2}}?)(meeting|call|sync|standup|stand-up|1:1|one-on-one|workshop|catch-up|check-in|kickoff|kick-off)\b")).unwrap()
});

/// The meeting clause in a sentence ("before our Monday meeting"), for removal from titles.
pub fn meeting_phrase(text: &str) -> Option<String> {
    RE_MEETING.find(&text.to_lowercase()).map(|m| m.as_str().to_string())
}
static RE_TIME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:by|at|before|until|no later than)\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm|a\.m\.|p\.m\.)?(?:\s|$|[.,;!?])|\b(noon|midday|midnight)\b").unwrap()
});
static RE_ISO: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(by|before|on|due|until)?\s*(\d{4})-(\d{2})-(\d{2})\b").unwrap());
static RE_MONTH_DAY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"\b(by|before|on|due|until|no later than)?\s*(?:{MONTHS}\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?|(\d{{1,2}})(?:st|nd|rd|th)?\s+(?:of\s+)?{MONTHS})\b")).unwrap()
});
static RE_WEEKDAY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(r"\b(by|before|on|this|next|until|due|no later than|end of)?\s*(?:the\s+)?(next\s+)?{WEEKDAY}\b(?:\s+(morning|afternoon|evening|night|eod|end of day))?")).unwrap()
});
static RE_REL_DAYS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(in|within|in the next)\s+(\d{1,2}|a|one|two|three|four|five|a couple of|a few)\s+(hours?|days?|business days?|working days?|weeks?)\b").unwrap()
});
static RE_SIMPLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(by|before|until|due|no later than)?\s*(?:the\s+)?(today|tonight|this evening|this afternoon|this morning|tomorrow(?:\s+(?:morning|afternoon|evening|eod))?|end of (?:the )?day|eod|cob|close of business|end of (?:the |this )?week|eow|this week|next week|end of (?:the |this )?month|end of next week|day after tomorrow)\b").unwrap()
});
static RE_ASAP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(asap|as soon as possible|urgent(ly)?|right away|immediately|top priority)\b").unwrap());

pub fn has_urgency(text: &str) -> bool {
    RE_ASAP.is_match(&text.to_lowercase())
}

fn weekday_from(s: &str) -> Option<Weekday> {
    Some(match &s[..3] {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

fn month_from(s: &str) -> Option<u32> {
    let m = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    m.iter().position(|p| s.starts_with(p)).map(|i| i as u32 + 1)
}

fn small_number(s: &str) -> Option<i64> {
    match s {
        "a" | "one" => Some(1),
        "two" | "a couple of" => Some(2),
        "three" | "a few" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        _ => s.parse().ok(),
    }
}

pub fn local_to_utc(tz: Tz, date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&date.and_time(time)) {
        chrono::LocalResult::Single(d) => d.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(a, _) => a.with_timezone(&Utc),
        // Inside a DST gap: shift forward an hour.
        chrono::LocalResult::None => tz
            .from_local_datetime(&(date.and_time(time) + Duration::hours(1)))
            .earliest()
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&date.and_time(time))),
    }
}

/// Next date with the given weekday. `allow_today` lets "by Friday" said on a Friday mean today.
fn next_weekday(from: NaiveDate, wd: Weekday, allow_today: bool) -> NaiveDate {
    let cur = from.weekday().num_days_from_monday() as i64;
    let tgt = wd.num_days_from_monday() as i64;
    let mut delta = (tgt - cur).rem_euclid(7);
    if delta == 0 && !allow_today {
        delta = 7;
    }
    from + Duration::days(delta)
}

fn add_business_days(mut d: NaiveDate, n: i64) -> NaiveDate {
    let mut left = n;
    while left > 0 {
        d += Duration::days(1);
        if d.weekday().num_days_from_monday() < 5 {
            left -= 1;
        }
    }
    d
}

fn end_of_week(d: NaiveDate) -> NaiveDate {
    let wd = d.weekday().num_days_from_monday() as i64;
    if wd <= 4 {
        d + Duration::days(4 - wd)
    } else {
        d // weekend: "end of week" already passed; caller treats as today
    }
}

fn last_day_of_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 { (d.year() + 1, 1) } else { (d.year(), d.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).unwrap() - Duration::days(1)
}

fn deadline_word(w: Option<&str>) -> bool {
    matches!(w.map(|s| s.trim()), Some("by" | "before" | "until" | "due" | "no later than" | "end of"))
}

/// Parse an explicit clock time ("by 3pm", "at 15:30", "noon").
fn parse_time(text: &str) -> Option<NaiveTime> {
    let c = RE_TIME.captures(text)?;
    if let Some(word) = c.get(4) {
        return match word.as_str() {
            "noon" | "midday" => NaiveTime::from_hms_opt(12, 0, 0),
            _ => NaiveTime::from_hms_opt(23, 59, 0),
        };
    }
    let mut h: u32 = c.get(1)?.as_str().parse().ok()?;
    let m: u32 = c.get(2).map(|m| m.as_str().parse().unwrap_or(0)).unwrap_or(0);
    let ampm = c.get(3).map(|m| m.as_str().replace('.', ""));
    match ampm.as_deref() {
        Some("pm") if h < 12 => h += 12,
        Some("am") if h == 12 => h = 0,
        None if c.get(2).is_none() && h <= 7 => h += 12, // "by 5" in business context = 17:00
        None if c.get(2).is_none() && h > 12 => {}
        None if c.get(2).is_none() => return None, // "by 9" is too ambiguous without am/pm or minutes
        _ => {}
    }
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Interpret the first deadline-like expression in `text`, relative to `reference`.
pub fn interpret(text: &str, reference: DateTime<Utc>, tz: Tz, work: WorkDay) -> Option<DeadlineGuess> {
    let t = text.to_lowercase().replace('\u{2019}', "'");
    let ref_local = reference.with_timezone(&tz);
    let today = ref_local.date_naive();
    let explicit_time = parse_time(&t);

    // Meeting-relative ("before our Monday meeting") — date resolved later against the calendar.
    let mut before_meeting = false;
    let mut meeting_weekday = None;
    let mut meeting_hint: Option<String> = None;
    if let Some(c) = RE_MEETING.captures(&t) {
        if matches!(c.get(1).map(|m| m.as_str()), Some("before" | "ahead of" | "prior to" | "for" | "by")) {
            before_meeting = true;
            meeting_weekday = c.get(2).and_then(|m| weekday_from(m.as_str()));
            let words = format!("{} {}", c.get(3).map(|m| m.as_str()).unwrap_or(""), c.get(4).map(|m| m.as_str()).unwrap_or(""));
            let hint: Vec<&str> = words.split_whitespace().filter(|w| !matches!(*w, "meeting" | "call" | "sync" | "the" | "our" | "weekly" | "next" | "upcoming")).collect();
            if !hint.is_empty() {
                meeting_hint = Some(hint.join(" "));
            }
        }
    }

    let mk = |date: NaiveDate, default_time: NaiveTime, phrase: &str, explicit: bool| {
        let time = explicit_time.unwrap_or(default_time);
        DeadlineGuess {
            due_at: local_to_utc(tz, date, time),
            phrase: phrase.trim().to_string(),
            kind: if explicit { "explicit".into() } else { "inferred".into() },
            before_meeting,
            meeting_weekday,
            meeting_hint: meeting_hint.clone(),
        }
    };

    // ISO dates.
    if let Some(c) = RE_ISO.captures(&t) {
        let y = c[2].parse().ok()?;
        let m = c[3].parse().ok()?;
        let d = c[4].parse().ok()?;
        if let Some(date) = NaiveDate::from_ymd_opt(y, m, d) {
            let before = c.get(1).map(|m| m.as_str()) == Some("before");
            return Some(mk(date, if before { work.start } else { work.end }, c.get(0)?.as_str(), true));
        }
    }

    // "October 22", "22nd Oct".
    if let Some(c) = RE_MONTH_DAY.captures(&t) {
        let (mon, day) = if let (Some(mo), Some(d)) = (c.get(2), c.get(3)) {
            (month_from(mo.as_str()), d.as_str().parse::<u32>().ok())
        } else {
            (c.get(5).and_then(|m| month_from(m.as_str())), c.get(4).and_then(|d| d.as_str().parse::<u32>().ok()))
        };
        if let (Some(mon), Some(day)) = (mon, day) {
            let mut date = NaiveDate::from_ymd_opt(today.year(), mon, day);
            if let Some(d0) = date {
                if d0 < today - Duration::days(7) {
                    date = NaiveDate::from_ymd_opt(today.year() + 1, mon, day);
                }
            }
            if let Some(date) = date {
                // "may" is also a verb: only accept "may" with a preposition or explicit day.
                let word = c.get(0)?.as_str();
                if !(word.contains("may") && c.get(1).is_none()) {
                    let pre = c.get(1).map(|m| m.as_str());
                    // A calendar date is always an explicit date.
                    let time = if pre == Some("before") { work.start } else { work.end };
                    return Some(mk(date, time, word, true));
                }
            }
        }
    }

    // Relative periods.
    if let Some(c) = RE_REL_DAYS.captures(&t) {
        let n = small_number(&c[2]).unwrap_or(1);
        let unit = &c[3];
        let phrase = c.get(0)?.as_str();
        if unit.starts_with("hour") {
            return Some(DeadlineGuess {
                due_at: reference + Duration::hours(n),
                phrase: phrase.into(),
                kind: "explicit".into(),
                before_meeting,
                meeting_weekday,
                meeting_hint: meeting_hint.clone(),
            });
        }
        let date = if unit.contains("business") || unit.contains("working") {
            add_business_days(today, n)
        } else if unit.starts_with("week") {
            today + Duration::days(7 * n)
        } else {
            today + Duration::days(n)
        };
        return Some(mk(date, work.end, phrase, true));
    }

    // Today / tomorrow / end of week ...
    if let Some(c) = RE_SIMPLE.captures(&t) {
        let pre = c.get(1).map(|m| m.as_str());
        let w = c.get(2)?.as_str();
        let phrase = c.get(0)?.as_str();
        let explicit = deadline_word(pre) || w.starts_with("end of") || matches!(w, "eod" | "cob" | "close of business" | "eow");
        let noon = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let (date, time, explicit) = match w {
            "today" | "end of day" | "end of the day" | "eod" | "cob" | "close of business" => (today, work.end, explicit || w != "today"),
            "this afternoon" => (today, NaiveTime::from_hms_opt(17, 0, 0).unwrap(), explicit),
            "this morning" => (today, noon, explicit),
            "tonight" | "this evening" => (today, NaiveTime::from_hms_opt(21, 0, 0).unwrap(), explicit),
            "day after tomorrow" => (today + Duration::days(2), work.end, explicit),
            "tomorrow morning" => (today + Duration::days(1), noon, explicit),
            "tomorrow afternoon" | "tomorrow eod" | "tomorrow evening" => (today + Duration::days(1), work.end, explicit),
            "tomorrow" => (today + Duration::days(1), work.end, explicit),
            "next week" => (end_of_week(today + Duration::days(7)), work.end, false),
            "end of next week" => (end_of_week(today + Duration::days(7)), work.end, true),
            "this week" => (end_of_week(today), work.end, explicit),
            _ if w.contains("week") || w == "eow" => (end_of_week(today), work.end, true),
            _ if w.contains("month") => (last_day_of_month(today), work.end, true),
            _ => return None,
        };
        let time = if pre == Some("before") && matches!(w, "tomorrow") { work.start } else { time };
        return Some(mk(date, time, phrase, explicit));
    }

    // Weekdays.
    if let Some(c) = RE_WEEKDAY.captures(&t) {
        let pre = c.get(1).map(|m| m.as_str().trim());
        let next = c.get(2).is_some() || pre == Some("next");
        let wd = weekday_from(c.get(3)?.as_str())?;
        // Skip "mon"/"sun"/"sat"/"wed" false positives unless they are real day words.
        let raw = c.get(3)?.as_str();
        if raw.len() <= 3 && pre.is_none() && !next {
            return None;
        }
        let mut date = next_weekday(today, wd, !next && pre != Some("before"));
        if next {
            // "next Friday" said early in a week means Friday of the following week.
            let same_week = date.iso_week() == today.iso_week();
            if same_week {
                date += Duration::days(7);
            }
        }
        let part = c.get(4).map(|m| m.as_str());
        let default_time = match (pre, part) {
            (Some("before"), _) => work.start,
            (_, Some("morning")) => NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            (_, Some("afternoon" | "eod" | "end of day")) => work.end,
            (_, Some("evening" | "night")) => NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            _ => work.end,
        };
        if before_meeting {
            // The meeting itself fixes the time; the weekday is just which meeting.
            return Some(mk(date, work.start, c.get(0)?.as_str(), true));
        }
        return Some(mk(date, default_time, c.get(0)?.as_str(), deadline_word(pre) || pre == Some("on")));
    }

    // Only a time ("by 3pm") → today if still ahead, else tomorrow.
    if let Some(time) = explicit_time {
        let mut date = today;
        if local_to_utc(tz, today, time) <= reference {
            date += Duration::days(1);
        }
        let phrase = RE_TIME.find(&t).map(|m| m.as_str().to_string()).unwrap_or_default();
        return Some(DeadlineGuess {
            due_at: local_to_utc(tz, date, time),
            phrase,
            kind: "explicit".into(),
            before_meeting,
            meeting_weekday,
            meeting_hint: meeting_hint.clone(),
        });
    }

    if before_meeting {
        // "before the meeting" with no day: the caller links the next matching meeting.
        return Some(DeadlineGuess {
            due_at: reference + Duration::days(1),
            phrase: RE_MEETING.find(&t).map(|m| m.as_str().to_string()).unwrap_or_default(),
            kind: "inferred".into(),
            before_meeting,
            meeting_weekday,
            meeting_hint: meeting_hint.clone(),
        });
    }
    None
}

/// A gentle suggested deadline for commitments without one (always flagged as a suggestion).
pub fn suggest(task_type: &str, reference: DateTime<Utc>, tz: Tz, work: WorkDay) -> DateTime<Utc> {
    let today = reference.with_timezone(&tz).date_naive();
    let days = match task_type {
        "reply" | "followup" => 1,
        "approval" | "review" => 2,
        _ => 3,
    };
    local_to_utc(tz, add_business_days(today, days), work.end)
}

/// Is `at` inside the user's working time?
pub fn is_working_time(at: DateTime<Utc>, tz: Tz, days: &[u32], work: WorkDay, holidays: &[NaiveDate]) -> bool {
    let l = at.with_timezone(&tz);
    let wd = l.weekday().number_from_monday();
    days.contains(&wd) && !holidays.contains(&l.date_naive()) && l.time() >= work.start && l.time() < work.end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn local(g: &DeadlineGuess, tz: Tz) -> String {
        g.due_at.with_timezone(&tz).format("%a %Y-%m-%d %H:%M").to_string()
    }
    const TZ: Tz = chrono_tz::Asia::Karachi;

    // Reference: Tuesday 2026-10-13 10:00 Karachi (05:00Z)
    const REF: &str = "2026-10-13T05:00:00Z";

    #[test]
    fn tomorrow_is_inferred_time_of_action() {
        let g = interpret("I'll share the revised report tomorrow.", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Wed 2026-10-14 17:30");
        assert_eq!(g.kind, "inferred");
    }

    #[test]
    fn before_thursday_is_explicit_start_of_day() {
        let g = interpret("Could you send me the updated analysis before Thursday?", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Thu 2026-10-15 09:00");
        assert_eq!(g.kind, "explicit");
    }

    #[test]
    fn by_friday_on_friday_is_today() {
        let fri = "2026-10-16T05:00:00Z";
        let g = interpret("Please finish it by Friday", r(fri), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Fri 2026-10-16 17:30");
    }

    #[test]
    fn next_friday_is_following_week() {
        let g = interpret("let's aim for next Friday", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Fri 2026-10-23 17:30");
    }

    #[test]
    fn meeting_relative() {
        let g = interpret("I'll prepare the slides before our Monday meeting.", r(REF), TZ, WorkDay::default()).unwrap();
        assert!(g.before_meeting);
        assert_eq!(g.meeting_weekday, Some(Weekday::Mon));
        assert_eq!(local(&g, TZ), "Mon 2026-10-19 09:00");
    }

    #[test]
    fn explicit_time_and_eod() {
        let g = interpret("Need this by 3pm", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Tue 2026-10-13 15:00");
        let g = interpret("send it EOD", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Tue 2026-10-13 17:30");
        assert_eq!(g.kind, "explicit");
    }

    #[test]
    fn month_day_and_rollover() {
        let g = interpret("due on October 22nd", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Thu 2026-10-22 17:30");
        let g = interpret("deadline is Jan 5", r(REF), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Tue 2027-01-05 17:30");
    }

    #[test]
    fn relative_business_days() {
        let fri = "2026-10-16T05:00:00Z";
        let g = interpret("within 2 business days", r(fri), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Tue 2026-10-20 17:30");
    }

    #[test]
    fn no_deadline_means_none() {
        assert!(interpret("Let me check with the finance team and get back to you.", r(REF), TZ, WorkDay::default()).is_none());
        assert!(interpret("I may look into it", r(REF), TZ, WorkDay::default()).is_none());
    }

    #[test]
    fn timezone_matters() {
        // 23:30 UTC Monday is already Tuesday in Karachi: "tomorrow" = Wednesday there.
        let g = interpret("I'll do it tomorrow", r("2026-10-12T23:30:00Z"), TZ, WorkDay::default()).unwrap();
        assert_eq!(local(&g, TZ), "Wed 2026-10-14 17:30");
        let ny: Tz = chrono_tz::America::New_York;
        let g = interpret("I'll do it tomorrow", r("2026-10-12T23:30:00Z"), ny, WorkDay::default()).unwrap();
        assert_eq!(local(&g, ny), "Tue 2026-10-13 17:30");
    }

    #[test]
    fn dst_gap_is_handled() {
        let ny: Tz = chrono_tz::America::New_York;
        let d = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let t = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        let u = local_to_utc(ny, d, t);
        assert_eq!(u.with_timezone(&ny).format("%H:%M").to_string(), "03:30");
    }
}

