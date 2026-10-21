//! Extraction quality evaluation on the labelled synthetic dataset.
//! Run: `cargo test -p kept-core --test eval -- --nocapture`
//! Prints precision / recall and per-case misses; fails if quality regresses below thresholds.

use chrono::{DateTime, Utc};
use kept_core::extract::{kind, rules};
use kept_core::models::SourceItem;
use kept_core::prefs::Preferences;
use serde_json::Value;

fn load(name: &str) -> Value {
    let p = format!("{}/../../sample-data/eval/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn rules_extractor_quality_dev_set() {
    let (p, r) = evaluate("extraction_cases.json", "DEV set (used while tuning)");
    // Regression guard (set from measured results; raise as the extractor improves).
    assert!(p >= 0.90, "precision regressed: {p}");
    assert!(r >= 0.90, "recall regressed: {r}");
}

#[test]
fn rules_extractor_quality_holdout_set() {
    // Reported, not tuned against. Only a very loose floor to catch breakage.
    let (p, r) = evaluate("extraction_holdout.json", "HOLD-OUT set (never tuned on)");
    assert!(p >= 0.5 && r >= 0.4, "holdout collapsed: p={p} r={r}");
}

fn evaluate(file: &str, label: &str) -> (f64, f64) {
    let data = load(file);
    let mut prefs = Preferences::default();
    prefs.user_name = data["me"]["name"].as_str().unwrap().into();
    prefs.user_emails = vec![data["me"]["email"].as_str().unwrap().into()];
    prefs.timezone = data["timezone"].as_str().unwrap().into();
    let reference: DateTime<Utc> = DateTime::parse_from_rfc3339(data["reference"].as_str().unwrap()).unwrap().with_timezone(&Utc);

    let (mut tp, mut fp, mut fn_) = (0, 0, 0);
    let (mut kind_ok, mut title_ok, mut due_ok, mut due_n, mut review_ok, mut review_n) = (0, 0, 0, 0, 0, 0);
    let mut misses: Vec<String> = vec![];
    let cases = data["cases"].as_array().unwrap();
    for c in cases {
        let from = c["from"].as_str().unwrap();
        let (sn, se) = kept_core::text::parse_address(from);
        let strs = |k: &str| c[k].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>()).unwrap_or_default();
        let item = SourceItem {
            id: c["id"].as_str().unwrap().into(),
            account_id: "eval".into(),
            provider: "eval".into(),
            kind: c["kind"].as_str().unwrap_or("email").into(),
            external_id: c["id"].as_str().unwrap().into(),
            thread_key: None,
            sender_name: sn,
            sender_email: Some(se.clone()),
            to: strs("to"),
            cc: strs("cc"),
            subject: c["subject"].as_str().map(|s| s.into()),
            body_text: Some(c["body"].as_str().unwrap().into()),
            snippet: None,
            url: None,
            sent_at: reference,
            is_from_me: prefs.is_me(&se),
            process_state: "pending".into(),
            process_note: None,
        };
        let predicted: Vec<_> = rules::extract_one(&prefs, &item, &[]).into_iter().filter(|x| kind::creates_task(&x.kind)).collect();
        let expected = c["expect"].as_array().unwrap();
        let mut used = vec![false; predicted.len()];
        for e in expected {
            let has: Vec<String> = e["title_has"].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap().to_lowercase()).collect()).unwrap_or_default();
            let found = predicted.iter().enumerate().find(|(i, p)| !used[*i] && (has.is_empty() || has.iter().any(|h| p.title.to_lowercase().contains(h) || p.evidence.to_lowercase().contains(h))));
            match found {
                Some((i, p)) => {
                    used[i] = true;
                    tp += 1;
                    if p.kind == e["kind"].as_str().unwrap() {
                        kind_ok += 1;
                    }
                    if has.iter().all(|h| p.title.to_lowercase().contains(h)) {
                        title_ok += 1;
                    }
                    if let Some(d) = e["due"].as_str() {
                        due_n += 1;
                        let got = p.due.as_ref().map(|g| g.kind.as_str()).unwrap_or("none");
                        if got == d {
                            due_ok += 1;
                        } else {
                            misses.push(format!("{}: due expected {d}, got {got}", c["id"]));
                        }
                    }
                    if e["review"].as_bool() == Some(true) {
                        review_n += 1;
                        if p.needs_confirmation || p.confidence < prefs.auto_accept_threshold {
                            review_ok += 1;
                        } else {
                            misses.push(format!("{}: should need review but conf={} ", c["id"], p.confidence));
                        }
                    }
                }
                None => {
                    fn_ += 1;
                    misses.push(format!("{}: MISSED {} — {:?} (got {:?})", c["id"], e["kind"], c["body"].as_str().unwrap(), predicted.iter().map(|p| &p.title).collect::<Vec<_>>()));
                }
            }
        }
        for (i, p) in predicted.iter().enumerate() {
            if !used[i] {
                fp += 1;
                misses.push(format!("{}: FALSE POSITIVE {} \"{}\"", c["id"], p.kind, p.title));
            }
        }
    }
    let precision = tp as f64 / (tp + fp).max(1) as f64;
    let recall = tp as f64 / (tp + fn_).max(1) as f64;
    println!("\n=== Rules extractor · {label} · {} labelled cases ===", cases.len());
    println!("detected correctly (TP): {tp}   false positives: {fp}   missed (FN): {fn_}");
    println!("precision: {:.1}%   recall: {:.1}%", precision * 100.0, recall * 100.0);
    println!("kind correct: {kind_ok}/{tp}   title contains key words: {title_ok}/{tp}   deadline kind correct: {due_ok}/{due_n}   routed to review when required: {review_ok}/{review_n}");
    for m in &misses {
        println!("  · {m}");
    }
    let report = serde_json::json!({"cases": cases.len(), "tp": tp, "fp": fp, "fn": fn_, "precision": precision, "recall": recall, "kind_ok": kind_ok, "title_ok": title_ok, "due_ok": due_ok, "due_n": due_n, "review_ok": review_ok, "review_n": review_n, "misses": misses});
    let _ = std::fs::write(format!("{}/../../target/eval_report_{}", env!("CARGO_MANIFEST_DIR"), file), serde_json::to_vec_pretty(&report).unwrap());
    (precision, recall)
}

/// Optional: measure the AI extractor on the same labelled sets with YOUR provider.
/// It sends the synthetic messages (no real data) to the provider and costs a few cents.
///   KEPT_EVAL_PROVIDER=anthropic KEPT_EVAL_API_KEY=sk-ant-... [KEPT_EVAL_MODEL=...] \
///   cargo test -p kept-core --test eval ai_extractor_quality -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn ai_extractor_quality() {
    let provider = std::env::var("KEPT_EVAL_PROVIDER").expect("set KEPT_EVAL_PROVIDER");
    let cfg = kept_core::ai::AiConfig {
        provider: provider.clone(),
        model: std::env::var("KEPT_EVAL_MODEL").unwrap_or_default(),
        base_url: std::env::var("KEPT_EVAL_BASE_URL").unwrap_or_default(),
        api_key: std::env::var("KEPT_EVAL_API_KEY").ok(),
        timeout: std::time::Duration::from_secs(60),
    };
    let http = kept_core::connectors::http::client();
    for file in ["extraction_cases.json", "extraction_holdout.json"] {
        let data = load(file);
        let mut prefs = Preferences::default();
        prefs.user_name = data["me"]["name"].as_str().unwrap().into();
        prefs.user_emails = vec![data["me"]["email"].as_str().unwrap().into()];
        prefs.timezone = data["timezone"].as_str().unwrap().into();
        let reference: DateTime<Utc> = DateTime::parse_from_rfc3339(data["reference"].as_str().unwrap()).unwrap().with_timezone(&Utc);
        let (mut tp, mut fp, mut fn_) = (0, 0, 0);
        for c in data["cases"].as_array().unwrap() {
            let (sn, se) = kept_core::text::parse_address(c["from"].as_str().unwrap());
            let strs = |k: &str| c[k].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>()).unwrap_or_default();
            let item = SourceItem {
                id: c["id"].as_str().unwrap().into(), account_id: "eval".into(), provider: "eval".into(), kind: c["kind"].as_str().unwrap_or("email").into(),
                external_id: c["id"].as_str().unwrap().into(), thread_key: None, sender_name: sn, sender_email: Some(se.clone()), to: strs("to"), cc: strs("cc"),
                subject: c["subject"].as_str().map(|s| s.into()), body_text: Some(c["body"].as_str().unwrap().into()), snippet: None, url: None,
                sent_at: reference, is_from_me: prefs.is_me(&se), process_state: "pending".into(), process_note: None,
            };
            let ctx = kept_core::extract::ExtractContext { prefs: &prefs, item: &item, thread: &[], open_tasks: &[], now: reference };
            let (prompt, _) = kept_core::extract::ai::build_user_prompt(&ctx);
            let out = kept_core::ai::complete(&http, &cfg, kept_core::extract::ai::SYSTEM_PROMPT, &prompt, 1500).await.expect("provider call");
            let v = kept_core::ai::extract_json(&out).unwrap_or(serde_json::json!({"items": []}));
            let pred: Vec<_> = kept_core::extract::ai::validate(&v, &ctx).items.into_iter().filter(|x| kind::creates_task(&x.kind)).collect();
            let exp = c["expect"].as_array().unwrap().len();
            let hit = pred.len().min(exp);
            tp += hit;
            fp += pred.len() - hit;
            fn_ += exp - hit;
        }
        let p = tp as f64 / (tp + fp).max(1) as f64;
        let r = tp as f64 / (tp + fn_).max(1) as f64;
        println!("AI ({provider}) on {file}: precision {:.1}%  recall {:.1}%  (tp {tp}, fp {fp}, fn {fn_})", p * 100.0, r * 100.0);
    }
}
