//! Text utilities: reply/quote stripping, sentence splitting, similarity, redaction.

use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

static RE_ON_WROTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?im)^\s*on .{4,120}(wrote|écrit|schrieb):\s*$").unwrap());
static RE_ORIGINAL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?im)^\s*(-{2,}\s*original message\s*-{2,}|_{5,}|from:\s.+\r?\n\s*(sent|date):\s)").unwrap());
static RE_SIG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^\s*(--\s*|sent from my \w+.*)$").unwrap());
static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t\u{a0}]+").unwrap());
static RE_SENT_SPLIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"([.!?])\s+|\n\s*\n|\n\s*[-*•]\s+").unwrap());

/// Keep only the newly written part of an email: drop quoted history and signatures.
pub fn strip_quoted(body: &str) -> String {
    let mut cut = body.len();
    for re in [&*RE_ON_WROTE, &*RE_ORIGINAL] {
        if let Some(m) = re.find(body) {
            cut = cut.min(m.start());
        }
    }
    let head = &body[..cut];
    let mut out = String::new();
    for line in head.lines() {
        if line.trim_start().starts_with('>') {
            continue;
        }
        if RE_SIG.is_match(line) {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

pub fn collapse_ws(s: &str) -> String {
    RE_WS.replace_all(s, " ").trim().to_string()
}

/// Split into sentences, keeping terminal punctuation.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0;
    for m in RE_SENT_SPLIT.find_iter(text) {
        let end = m.start() + m.as_str().trim_end().len();
        let s = collapse_ws(&text[last..end].replace('\n', " "));
        if s.len() > 2 {
            out.push(s);
        }
        last = m.end();
    }
    let tail = collapse_ws(&text[last..].replace('\n', " "));
    if tail.len() > 2 {
        out.push(tail);
    }
    out.into_iter().map(|s| s.trim_start_matches(['-', '*', '•', ' ']).to_string()).collect()
}

const STOP: &[&str] = &[
    "the", "a", "an", "to", "of", "and", "or", "for", "on", "in", "at", "by", "with", "me", "my", "you", "your", "i",
    "we", "our", "it", "this", "that", "is", "are", "be", "will", "ll", "can", "could", "would", "please", "before",
    "after", "about", "from", "up", "get", "back", "send", "share", "just", "so", "some", "any", "all", "us", "them",
    "tomorrow", "today", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "week", "next",
    "end", "day", "eod", "let", "check", "s", "re", "fw", "fwd", "task", "follow", "following", "updated", "revised",
    "new", "latest", "final", "draft", "do", "make", "need", "needs",
];

/// Very light stemming so "reports"/"report", "slides"/"slide" match.
fn stem(w: &str) -> String {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    for suf in ["ing", "ed", "es", "s"] {
        if w.len() > suf.len() + 3 && w.ends_with(suf) {
            return w[..w.len() - suf.len()].to_string();
        }
    }
    w.to_string()
}

pub fn keywords(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !STOP.contains(w))
        .map(stem)
        .filter(|w| w.len() > 1)
        .collect()
}

/// Jaccard similarity over keyword sets (0..1).
pub fn similarity(a: &str, b: &str) -> f64 {
    let ka = keywords(a);
    let kb = keywords(b);
    if ka.is_empty() || kb.is_empty() {
        return 0.0;
    }
    let inter = ka.intersection(&kb).count() as f64;
    let union = ka.union(&kb).count() as f64;
    inter / union
}

/// Overlap coefficient: good for "report" vs "revised Q3 report".
pub fn overlap(a: &str, b: &str) -> f64 {
    let ka = keywords(a);
    let kb = keywords(b);
    if ka.is_empty() || kb.is_empty() {
        return 0.0;
    }
    ka.intersection(&kb).count() as f64 / ka.len().min(kb.len()) as f64
}

/// Stable fingerprint of a task's meaning (sorted keywords).
pub fn fingerprint(title: &str) -> String {
    let mut k: Vec<_> = keywords(title).into_iter().collect();
    k.sort();
    k.join(" ")
}

pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

static RE_EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap());
static RE_PHONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\+?\d[\d\s().-]{8,}\d").unwrap());
static RE_CARD: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(?:\d[ -]?){13,19}\b").unwrap());
static RE_SECRETISH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\b(password|passcode|pwd|api[_ -]?key|secret|token|otp|pin)\b\s*[:=]?\s*\S+|\b(sk-[A-Za-z0-9_-]{12,}|xox[abpr]-[A-Za-z0-9-]{10,}|ya29\.[A-Za-z0-9_-]{20,}|eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)"#).unwrap()
});
static RE_URL_QUERY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(https?://[^\s?#]+)\?[^\s]+").unwrap());

/// Remove credentials always; optionally remove contact PII before text leaves the machine.
pub fn redact(s: &str, pii: bool) -> String {
    let mut out = RE_SECRETISH.replace_all(s, "[REDACTED-SECRET]").to_string();
    out = RE_URL_QUERY.replace_all(&out, "$1?[…]").to_string();
    out = RE_CARD.replace_all(&out, "[REDACTED-NUMBER]").to_string();
    if pii {
        out = RE_PHONE.replace_all(&out, "[PHONE]").to_string();
        out = RE_EMAIL.replace_all(&out, "[EMAIL]").to_string();
    }
    out
}

/// For logs: never print bodies; print length and hash prefix only.
pub fn log_safe(s: &str) -> String {
    format!("<{} chars #{}>", s.len(), &sha256_hex(s)[..8])
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

/// Turn "Sara Khan <sara@acme.com>" into (Some("Sara Khan"), "sara@acme.com").
pub fn parse_address(s: &str) -> (Option<String>, String) {
    let s = s.trim();
    if let (Some(a), Some(b)) = (s.rfind('<'), s.rfind('>')) {
        if a < b {
            let name = s[..a].trim().trim_matches('"').trim();
            return ((!name.is_empty()).then(|| name.to_string()), s[a + 1..b].trim().to_lowercase());
        }
    }
    (None, s.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_quotes_and_signatures() {
        let b = "Sounds good, I'll send it Friday.\n\nThanks\n--\nAli\n\nOn Mon, Oct 12, 2026 at 9:00 AM Sara <s@x.com> wrote:\n> Can you send the deck?";
        let s = strip_quoted(b);
        assert!(s.contains("I'll send it Friday"));
        assert!(!s.contains("send the deck"));
        assert!(!s.contains("Ali"));
    }

    #[test]
    fn splits_sentences() {
        let s = sentences("Hi team. I'll prepare the slides! Can you review?\n\n- Item one\n- Item two");
        assert_eq!(s.len(), 5, "{s:?}");
    }

    #[test]
    fn similarity_works() {
        assert!(overlap("Share the revised Q3 report", "Send Q3 report to Sara") >= 0.5);
        assert!(similarity("Prepare slides for Monday", "Book flights to Dubai") < 0.1);
    }

    #[test]
    fn redaction() {
        let r = redact("my password: hunter2 call +92 300 1234567 or a@b.com key sk-abcdefghijklmnop123", true);
        assert!(!r.contains("hunter2"));
        assert!(!r.contains("1234567"));
        assert!(!r.contains("a@b.com"));
        assert!(!r.contains("sk-abc"));
    }
}
