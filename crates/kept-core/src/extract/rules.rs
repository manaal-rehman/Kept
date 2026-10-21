//! Deterministic, offline extractor.
//!
//! It understands sentence-level intent from wording, author, recipients and tense.
//! It is intentionally conservative: anything uncertain is routed to review with a
//! reason, never silently assigned to the user.

use super::{kind, ExtractContext, Extracted};
use crate::dates::{self, WorkDay};
use crate::models::{Person, SourceItem};
use crate::prefs::{parse_hm, Preferences};
use crate::text::{self, sentences};
use once_cell::sync::Lazy;
use regex::Regex;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: Lazy<Regex> = Lazy::new(|| Regex::new($pat).unwrap());
    };
}

// --- my own commitments -------------------------------------------------------
re!(MY_COMMIT, r"\b(i'll|i will|i'm going to|i am going to|i shall|i can have|let me|i promise to|i'll make sure to|i'll make sure|i'll be sure to|i'll try to|i will try to|leave it with me,? i'll|i'll take care of|i'll handle|i'll get|i'll aim to|i plan to|i intend to)\s+");
re!(MY_NEGATED, r"\b(i won't|i will not|i can't|i cannot|i'm not going to|i am not going to|i don't think i'll|i couldn't)\b");
re!(HYPOTHETICAL_LEAD, r"\b(if|unless|in case|whether|maybe|perhaps|possibly|probably|hypothetically|ideally|assuming|suppose|supposing)\b");
re!(HEDGE_VERB, r"\b(i'd|i would|i could|i might|i may|should i|could i|would i|shall i)\b");
re!(LET_ME_KNOW, r"\blet me know\b");
re!(AGREEMENT, r"^(sure|yes|yep|yeah|ok|okay|sounds good|will do|on it|no problem|happy to|absolutely|of course|consider it done|can do|noted,? will do)\b");
re!(COMPLETION, r"\b(i've|i have|i just|just|i) (sent|shared|attached|uploaded|submitted|emailed|forwarded|finished|completed|pushed|published|delivered)\b|\b(please find attached|please see attached|attached is|attached are|attached you'll find|here is the|here's the|here are the|enclosed is|enclosed are)\b|^(done|sent|shared|submitted)\b");
re!(DRAFTY, r"\b(draft|first cut|first pass|for your review|for review|your feedback|for approval|for your approval|sign[- ]?off|let me know if (you'd like|you want|any) (changes|edits))\b");

// --- other people ---------------------------------------------------------------
re!(REQUEST, r"\b(can someone|could someone|can anyone|could anyone|could you|can you|would you|will you|could u|can u|please|pls|plz|kindly|i need you to|you need to|we need you to|i'd like you to|make sure you|don't forget to|remember to|would you mind|are you able to|can i get|could i get|can we get|could we get|need you to)\b");
re!(APPROVAL_ASK, r"\b(need(s)? your (approval|sign[- ]?off|go[- ]ahead|ok|decision)|waiting (on|for) your (approval|sign[- ]?off|decision|go[- ]ahead)|(please|can you|could you) (approve|sign off|give (the|your) go[- ]ahead)|for your approval|your approval (is|to)|still need your approval|approval from you)\b");
re!(APPROVAL_GIVEN, r"\b(approved|i approve|lgtm|looks good to me|looks great,? go ahead|go ahead and|you have my (approval|sign[- ]?off)|signed off|green light|good to proceed|happy for you to proceed|all good,? (go|proceed))\b");
re!(APPROVAL_NOT_GIVEN, r"\b(not approved|once approved|before approv|needs? (to be )?approv|pending approval|awaiting approval|can't approve|cannot approve|approval\?|not yet)\b");
re!(NUDGE, r"\b(following up|follow(?:ing)? up on|any update|any news on|any progress|just checking in|checking in on|gentle reminder|friendly reminder|quick reminder|circling back|still waiting|haven't received|have not received|haven't seen|you promised|you mentioned you('d| would)|you said you('d| would)|bumping this|as promised\?)\b");
re!(IMPLICIT, r"\b(you're|you are|you'll be|you will be) (handling|owning|leading|responsible for|in charge of|on point for|taking care of|covering|presenting|running|preparing|drafting|sending|putting together)\b|\b(assigned to you|over to you|your action item|action item for you|you own the|you're the owner|you're up for)\b");
re!(THEIR_COMMIT, r"\b(i'll|i will|we'll|we will|i'm going to|we're going to|i am going to|we are going to|let me)\s+");
// Deadline / obligation statements with no request verb ("Tomorrow is the deadline", "The report is due Friday",
// "You must submit…", "if you fail to deliver…").
re!(OBLIGATION, r"\b(is the deadline|deadline is|the deadline for|deadline:|is due|are due|due (by|on|before|tomorrow|today|this|next)|you must|you have to|you've got to|you are required to|you're required to|has to be (done|delivered|submitted|sent|ready|finished)|must be (done|delivered|submitted|sent|ready|finished)|expected (from you|by|before)|fail to (deliver|submit|send|complete|finish|meet)|failing to (deliver|submit|send|complete|finish|meet)|no later than|without fail)\b");
re!(OBLIGATION_OBJECT, r"(?i)\b(?:deliver|submit|send|complete|finish|hand in|provide)\s+(?:me\s+|us\s+)?((?:the|your|a|an|this|that)\s+[\w' -]{2,60}?)(?:\s+(?:by|before|on|tomorrow|today|or|and|,|\.)|$)");
re!(OBLIGATION_SUBJECT, r"(?i)^((?:the|your|our|this)\s+[\w' -]{2,60}?)\s+(?:is|are|has to be|must be)\s+(?:due|expected|needed)");
re!(CONSEQUENCE, r"\b(fired|terminated|disciplinary|final warning|escalat|consequences|penalt|urgent|critical|asap)");
re!(FEEDBACK_ASK, r"\b(would love|would appreciate|i'd love|i'd appreciate|keen to get|looking for|need) your (feedback|input|thoughts|comments|review|eyes)\b");
re!(ON_YOU, r"(?i)^(?:so\s+|and\s+)?(the\s+.+?|[a-z0-9 '-]+?)\s+(?:is|are)\s+(?:on you|yours|your responsibility|your call|with you)\b(.*)$");
re!(VOCATIVE, r"^@?([A-Za-z][A-Za-z.'-]{1,30})\s*[,:]\s");
re!(AT_MENTION, r"@([A-Za-z][\w.-]{1,30})");
re!(DELIVERABLE, r"(?i)\b((?:[\w'-]+\s){0,3}?(report|deck|slides|slide deck|presentation|analysis|proposal|document|doc|spreadsheet|sheet|draft|plan|summary|budget|contract|memo|minutes|invoice|estimate|quote|figures|numbers|forecast|agenda|brief|spec|specification|design|mockups?|notes|review|update|projections|roadmap|timeline|pricing|po|sow|receipts?|invoices?|files?|results|data|details|figures|photos|checklist|pr|pull request))\b");

const CUT_AT: &[&str] = &[", i'll ", ", i will ", ", we'll ", ", so ", ", which ", " because ", " so that ", " so we ", " so you ", " so i ", " so they ", " so he ", " so she ", " so it ", " so the ", " once ", " if ", " unless ", " as soon as ", " when ", " but ", " though ", " – ", " — ", " - "];
const TRIM_LEAD: &[&str] = &["please ", "kindly ", "just ", "also ", "quickly ", "go ahead and ", "be able to ", "try to ", "make sure to ", "then "];

pub fn workday(p: &Preferences) -> WorkDay {
    WorkDay {
        start: parse_hm(&p.work_start).unwrap_or(WorkDay::default().start),
        end: parse_hm(&p.work_end).unwrap_or(WorkDay::default().end),
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'")
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn first_name(p: &Option<Person>) -> String {
    p.as_ref()
        .and_then(|p| p.name.clone().or_else(|| p.email.as_ref().map(|e| e.split('@').next().unwrap_or("").to_string())))
        .and_then(|n| n.split_whitespace().next().map(|s| s.to_string()))
        .unwrap_or_else(|| "them".into())
}

fn clean_subject(s: Option<&str>) -> String {
    let mut s = s.unwrap_or("").trim().to_string();
    loop {
        let l = s.to_lowercase();
        let stripped = ["re:", "fw:", "fwd:", "re :", "aw:", "[external]"].iter().find(|p| l.starts_with(*p));
        match stripped {
            Some(p) => s = s[p.len()..].trim().to_string(),
            None => break,
        }
    }
    s
}

/// Words (case-preserving) after byte offset `start`, cut at clause boundaries, with deadline words removed.
fn action_phrase(orig: &str, start: usize, deadline_phrase: Option<&str>) -> String {
    let start = start.min(orig.len());
    if !orig.is_char_boundary(start) {
        return String::new();
    }
    let mut p = orig[start..].to_string();
    // Clause cut.
    let lower = norm(&p);
    let mut cut = p.len();
    for c in CUT_AT {
        if let Some(i) = lower.find(c) {
            cut = cut.min(i);
        }
    }
    for c in ['?', '!', ';', '\n'] {
        if let Some(i) = p.find(c) {
            cut = cut.min(i);
        }
    }
    if let Some(i) = p.find(". ") {
        cut = cut.min(i);
    }
    p.truncate(cut);
    let mut p = p.trim().trim_end_matches(['.', ',', ':']).to_string();
    if let Some(ph) = deadline_phrase {
        let l = norm(&p);
        if let Some(i) = l.find(&norm(ph)) {
            let end = (i + ph.len()).min(p.len());
            p = format!("{}{}", &p[..i], &p[end..]);
        }
    }
    let mut p = text::collapse_ws(&p);
    // Strip trailing dangling prepositions / time words.
    loop {
        let l = norm(&p);
        let trimmed = [" by", " before", " on", " until", " for", " asap", " at", " this", " next", " today", " tomorrow", " later", " soon", " instead", " too", " as well", " please", ","]
            .iter()
            .find(|w| l.ends_with(*w));
        match trimmed {
            Some(w) => p.truncate(p.len() - w.len()),
            None => break,
        }
    }
    loop {
        let l = norm(&p);
        match TRIM_LEAD.iter().find(|w| l.starts_with(*w)) {
            Some(w) => p = p[w.len()..].to_string(),
            None => break,
        }
    }
    p.trim().to_string()
}

/// Rewrite pronouns so the title reads from my point of view.
fn my_view(phrase: &str, counterpart: &str, requester_view: bool) -> String {
    let mut words: Vec<String> = phrase.split_whitespace().map(|s| s.to_string()).collect();
    let n = words.len();
    let mut moved_to: Option<String> = None;
    for i in 0..n {
        let w = norm(&words[i]);
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
        let punct: String = words[i].chars().rev().take_while(|c| !c.is_alphanumeric()).collect::<String>().chars().rev().collect();
        if requester_view {
            // Text written by the counterpart: "me"/"us" = them, "you"/"your" = me.
            match bare {
                "me" | "us" => {
                    // "send me the deck" -> "send the deck to Sara"
                    if i == 1 && matches!(norm(&words[0]).as_str(), "send" | "give" | "email" | "share" | "show" | "forward" | "pass" | "tell" | "get") && n > 2 {
                        moved_to = Some(counterpart.to_string());
                        words[i] = String::new();
                    } else {
                        words[i] = format!("{counterpart}{punct}");
                    }
                }
                "my" | "our" => words[i] = format!("{counterpart}'s{punct}"),
                "your" => words[i] = format!("my{punct}"),
                "you" => words[i] = format!("me{punct}"),
                "yourself" => words[i] = format!("myself{punct}"),
                _ => {}
            }
        } else {
            // Text I wrote: "you"/"your" = counterpart.
            match bare {
                "you" => words[i] = format!("{counterpart}{punct}"),
                "your" => words[i] = format!("{counterpart}'s{punct}"),
                "yours" => words[i] = format!("{counterpart}'s{punct}"),
                "y'all" | "you all" => words[i] = format!("everyone{punct}"),
                _ => {}
            }
        }
    }
    let mut s = words.into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if let Some(to) = moved_to {
        s = format!("{s} to {to}");
    }
    s
}

fn infer_type(phrase: &str, sentence: &str) -> String {
    let l = norm(&format!("{phrase} {sentence}"));
    let p = norm(phrase);
    if Regex::new(r"\b(approv|sign[- ]?off|go[- ]ahead|green light)").unwrap().is_match(&p) {
        return "approval".into();
    }
    if Regex::new(r"^(prepare|prep|get ready)\b").unwrap().is_match(&p) && l.contains("meeting") {
        return "prep".into();
    }
    if DELIVERABLE.is_match(&p) && Regex::new(r"^(send|share|prepare|write|draft|finish|finalize|finalise|complete|revise|update|put together|create|build|deliver|submit|produce|compile)").unwrap().is_match(&p) {
        return "deliverable".into();
    }
    if Regex::new(r"^(review|look over|check over|proofread|go through)").unwrap().is_match(&p) {
        return "review".into();
    }
    if Regex::new(r"^(check with|follow up|ping|chase|remind|ask|confirm with|circle back|reach out|contact|loop in)").unwrap().is_match(&p) {
        return "followup".into();
    }
    if Regex::new(r"^(reply|respond|get back|answer|let .* know|confirm)").unwrap().is_match(&p) {
        return "reply".into();
    }
    "action".into()
}

fn deliverable_of(phrase: &str) -> Option<String> {
    DELIVERABLE.captures(phrase).map(|c| {
        let s = c.get(1).unwrap().as_str().trim();
        let s = s.trim_start_matches(|c: char| !c.is_alphanumeric());
        // drop leading verbs/articles captured in the window
        let words: Vec<&str> = s.split_whitespace().collect();
        let skip = words
            .iter()
            .take_while(|w| matches!(norm(w).as_str(), "send" | "share" | "prepare" | "the" | "a" | "an" | "my" | "our" | "finish" | "revise" | "update" | "write" | "draft" | "to" | "and" | "me" | "you" | "your" | "with" | "get" | "on" | "of"))
            .count();
        words[skip.min(words.len().saturating_sub(1))..].join(" ")
    })
}

fn criteria(task_type: &str, phrase: &str, sentence: &str, counterpart: &str) -> Option<String> {
    let p = norm(phrase);
    let needs_approval = DRAFTY.is_match(&norm(sentence)) && norm(sentence).contains("approv");
    let base = match task_type {
        "approval" => Some(format!("Decision (approve / reject) communicated to {counterpart}")),
        "reply" => Some(format!("Reply sent to {counterpart}")),
        "followup" => Some(format!("Outcome communicated to {counterpart}")),
        "prep" => Some("Preparation finished before the meeting starts".into()),
        _ if Regex::new(r"^(send|share|email|forward|submit|deliver)").unwrap().is_match(&p) => {
            Some(format!("{} sent to {counterpart}", deliverable_of(phrase).map(|d| cap(&d)).unwrap_or_else(|| "It".into())))
        }
        "deliverable" => Some(format!("{} finished", deliverable_of(phrase).map(|d| cap(&d)).unwrap_or_else(|| "Deliverable".into()))),
        _ => None,
    };
    base.map(|b| if needs_approval { format!("{b} and approved") } else { b })
}

fn base_item(ctx: &ExtractContext, k: &str, sentence: &str) -> Extracted {
    Extracted {
        kind: k.into(),
        evidence: sentence.to_string(),
        my_role: "owner".into(),
        counterpart: ctx.counterpart(),
        urgent: dates::has_urgency(sentence),
        extractor: "rules".into(),
        ..Default::default()
    }
}

fn due_for(ctx: &ExtractContext, sentence: &str) -> Option<dates::DeadlineGuess> {
    dates::interpret(sentence, ctx.item.sent_at, ctx.prefs.tz(), workday(ctx.prefs))
}

/// Build a task-like item from an action phrase.
/// Deadline from the action clause first (so another clause's date isn't borrowed), then the lead-in.
fn clause_due(ctx: &ExtractContext, sentence: &str, start: usize) -> Option<dates::DeadlineGuess> {
    let clause = action_phrase(sentence, start, None);
    let raw_clause = if sentence.is_char_boundary(start.min(sentence.len())) { &sentence[start.min(sentence.len())..] } else { sentence };
    // action_phrase strips trailing time words, so look at the raw clause up to its cut point.
    let cut = raw_clause.find(['?', '!', ';']).unwrap_or(raw_clause.len());
    let lower = norm(&raw_clause[..cut]);
    let mut end = lower.len();
    for c in CUT_AT {
        if let Some(i) = lower.find(c) {
            end = end.min(i);
        }
    }
    let raw = &raw_clause[..end.min(cut)];
    due_for(ctx, raw)
        .or_else(|| if clause.is_empty() { None } else { due_for(ctx, &clause) })
        .or_else(|| if start > 0 && sentence.is_char_boundary(start) { due_for(ctx, &sentence[..start]) } else { None })
}

fn task_item(ctx: &ExtractContext, k: &str, sentence: &str, start: usize, requester_view: bool, confidence: f64) -> Option<Extracted> {
    let due = clause_due(ctx, sentence, start);
    let mut phrase = action_phrase(sentence, start, due.as_ref().map(|d| d.phrase.as_str()));
    // "Prepare the slides before our Monday meeting" → title "Prepare the slides" (meeting is linked separately).
    if let Some(mp) = dates::meeting_phrase(&phrase) {
        if let Some(i) = norm(&phrase).find(&mp) {
            let end = (i + mp.len()).min(phrase.len());
            phrase = text::collapse_ws(&format!("{}{}", &phrase[..i], &phrase[end..]));
        }
    }
    let counterpart = ctx.counterpart();
    let cp = first_name(&counterpart);
    let mut phrase = my_view(&phrase, &cp, requester_view);
    // "Share the revised report" → "Share the revised report with Sara" when I wrote to one person.
    if !requester_view && counterpart.is_some() && ctx.other_to_count() == 1 && !phrase.contains(&cp) {
        let l = norm(&phrase);
        if l.starts_with("share ") {
            phrase = format!("{phrase} with {cp}");
        } else if l.starts_with("send ") || l.starts_with("email ") || l.starts_with("forward ") {
            phrase = format!("{phrase} to {cp}");
        }
    }
    if phrase.split_whitespace().count() < 2 {
        return None;
    }
    let mut it = base_item(ctx, k, sentence);
    it.task_type = infer_type(&phrase, sentence);
    it.title = text::truncate(&cap(&phrase), 100);
    it.expected_deliverable = deliverable_of(&phrase).map(|d| cap(&d));
    it.completion_criteria = criteria(&it.task_type, &phrase, sentence, &cp);
    it.due_phrase = due.as_ref().map(|d| d.phrase.clone());
    it.due = due;
    it.confidence = confidence + if it.due.is_some() { 0.05 } else { 0.0 };
    it.project = None;
    Some(it)
}

pub fn extract(ctx: &ExtractContext) -> Vec<Extracted> {
    let text = ctx.new_text();
    let sents = sentences(&text);
    let mut out: Vec<Extracted> = Vec::new();
    let from_me = ctx.item.is_from_me;
    let subject = clean_subject(ctx.item.subject.as_deref());
    let sender = ctx.sender_first_name();
    let is_channel_post = ctx.item.kind == "chat" && ctx.item.to.is_empty();

    for (i, s) in sents.iter().enumerate() {
        let s = &s.replace(['\u{2019}', '\u{2018}'], "'");
        let l = norm(s);
        if l.len() != s.len() {
            continue; // unusual casing changes byte offsets; skip rather than mis-slice
        }
        if from_me {
            if let Some(m) = COMPLETION.find(&l) {
                // "I'll send..." is not completion; "I sent" is.
                if !MY_COMMIT.is_match(&l[..m.start()]) {
                    let mut it = base_item(ctx, kind::COMPLETION_EVIDENCE, s);
                    it.title = text::truncate(s, 120);
                    it.completion_stage = Some(if DRAFTY.is_match(&l) { "draft_sent".into() } else { "sent".into() });
                    it.confidence = 0.75;
                    it.expected_deliverable = deliverable_of(s).map(|d| cap(&d));
                    out.push(it);
                    continue;
                }
            }
            if MY_NEGATED.is_match(&l) {
                continue;
            }
            if let Some(m) = MY_COMMIT.find(&l) {
                if LET_ME_KNOW.is_match(&l[m.start()..]) {
                    continue;
                }
                let lead = &l[..m.start()];
                if HEDGE_VERB.is_match(&l) && !l.contains("i'll") && !l.contains("i will") {
                    continue; // "I could send it" is an offer, not a promise
                }
                let conditional = HYPOTHETICAL_LEAD.is_match(lead) || l.starts_with("if ");
                let tried = m.as_str().contains("try");
                let conf = if conditional { 0.45 } else if tried { 0.65 } else { 0.85 };
                if let Some(mut it) = task_item(ctx, kind::MY_COMMITMENT, s, m.end(), false, conf) {
                    if conditional {
                        it.needs_confirmation = true;
                        it.ambiguity = Some("Conditional wording — this may not be a firm commitment.".into());
                    }
                    if is_channel_post || ctx.item.to.is_empty() {
                        it.counterpart = None;
                        it.title = it.title.replace(" them", " the team");
                    }
                    out.push(it);
                }
                continue;
            }
            // "Sure, will do" in reply to a request → commit to that request.
            if i <= 1 && AGREEMENT.is_match(&l) && s.split_whitespace().count() <= 12 {
                if let Some(prev) = ctx.thread.iter().rev().find(|m| !m.is_from_me) {
                    let sub = ExtractContext { prefs: ctx.prefs, item: prev, thread: &[], open_tasks: ctx.open_tasks, now: ctx.now };
                    let req = extract(&sub).into_iter().find(|x| matches!(x.kind.as_str(), kind::REQUEST_TO_ME | kind::APPROVAL_REQUEST | kind::IMPLICIT_ASSIGNMENT));
                    if let Some(mut r) = req {
                        r.kind = kind::MY_COMMITMENT.into();
                        r.description = Some(format!("You agreed (\"{s}\") to: \"{}\"", r.evidence));
                        r.evidence = s.clone();
                        r.confidence = (r.confidence + 0.1).min(0.95);
                        r.needs_confirmation = false;
                        r.ambiguity = None;
                        out.push(r);
                    }
                }
            }
            continue;
        }

        // ---- messages written by someone else --------------------------------
        // Directed at somebody else by name? ("Omar, can you send…" when Omar is another recipient)
        if let Some(c) = VOCATIVE.captures(s) {
            let name = c[1].to_lowercase();
            let other_recipient = ctx.item.to.iter().chain(ctx.item.cc.iter()).any(|a| {
                let (n, e) = text::parse_address(a);
                !ctx.prefs.is_me(&e)
                    && (n.as_deref().and_then(|n| n.split_whitespace().next()).map(|f| f.to_lowercase() == name).unwrap_or(false)
                        || e.split(['@', '.']).next().map(|l| l == name).unwrap_or(false))
            });
            if other_recipient && !ctx.prefs.first_names().contains(&name) {
                continue;
            }
        }
        // @mentions of other people only.
        let mentions: Vec<String> = AT_MENTION.captures_iter(s).map(|c| c[1].to_lowercase()).collect();
        if !mentions.is_empty() && !ctx.mentions_me(s) {
            continue;
        }

        let directed = ctx.addressed_to_me() || ctx.mentions_me(s) || (ctx.item.kind == "chat" && !is_channel_post);
        if ctx.only_cc() && !ctx.mentions_me(s) {
            continue; // merely copied
        }
        if is_channel_post && !ctx.mentions_me(s) {
            continue;
        }
        let multi = ctx.other_to_count() >= 1 && !ctx.mentions_me(s);

        if APPROVAL_GIVEN.is_match(&l) && !APPROVAL_NOT_GIVEN.is_match(&l) && !l.contains('?') && !REQUEST.is_match(&l) {
            let mut it = base_item(ctx, kind::APPROVAL_RECEIVED, s);
            it.title = text::truncate(s, 120);
            it.confidence = 0.8;
            out.push(it);
            continue;
        }
        if NUDGE.is_match(&l) && directed {
            let mut it = base_item(ctx, kind::FOLLOWUP_NUDGE, s);
            let object = Regex::new(r"(?i)\b(?:on|about|re:?|regarding)\s+(?:the\s+)?(.+)$").unwrap().captures(s).map(|c| c[1].to_string());
            let object = object
                .map(|o| {
                    let o = Regex::new(r"(?i)\s+(you|that you|which you)\s+(promised|mentioned|said|were going).*$").unwrap().replace(&o, "").to_string();
                    let o = o.split([',', ';']).next().unwrap_or(&o).split(" — ").next().unwrap_or(&o).split(" - ").next().unwrap_or(&o).to_string();
                    o.trim_end_matches(['.', '?', '!']).to_string()
                })
                .filter(|o| o.split_whitespace().count() >= 1 && o.len() > 2)
                .unwrap_or_else(|| subject.clone());
            it.title = text::truncate(&format!("Respond to {sender}'s follow-up: {object}"), 100);
            it.task_type = "followup".into();
            it.expected_deliverable = deliverable_of(&object).map(|d| cap(&d));
            it.confidence = 0.8;
            it.urgent = true;
            it.completion_criteria = Some(format!("{sender} has what was promised, or a reply with a new date"));
            it.description = Some(format!("{sender} is chasing something. Kept will try to link this to the original commitment."));
            out.push(it);
            continue;
        }
        if APPROVAL_ASK.is_match(&l) && directed {
            let mut it = base_item(ctx, kind::APPROVAL_REQUEST, s);
            let obj = if subject.is_empty() { text::truncate(s, 60) } else { subject.clone() };
            it.title = text::truncate(&format!("Give approval decision: {obj}"), 100);
            it.task_type = "approval".into();
            it.completion_criteria = Some(format!("Approval or rejection communicated to {sender}"));
            it.due = due_for(ctx, s).map(|mut d| {
                d.kind = "explicit".into();
                d
            });
            it.due_phrase = it.due.as_ref().map(|d| d.phrase.clone());
            it.confidence = if multi { 0.55 } else { 0.85 };
            it.description = Some(format!("{sender} is waiting on your decision before moving forward."));
            if multi {
                it.needs_confirmation = true;
                it.ambiguity = Some("Several people were addressed; the approver may not be you.".into());
            }
            out.push(it);
            continue;
        }
        if OBLIGATION.is_match(&l) && directed && !REQUEST.is_match(&l) && !l.contains('?') {
            // Only an obligation if it's about something to do — not "the invoice is due to you".
            let due = due_for(ctx, s).map(|mut d| {
                d.kind = "explicit".into();
                d
            });
            let object = OBLIGATION_OBJECT
                .captures(s)
                .or_else(|| OBLIGATION_SUBJECT.captures(s))
                .map(|c| c[1].trim().to_string())
                .filter(|o| !o.to_lowercase().starts_with("the deadline"));
            let generic_subject = subject.is_empty() || matches!(subject.to_lowercase().as_str(), "deadline" | "urgent" | "reminder" | "important" | "re" | "fyi" | "update" | "hi" | "hello");
            let (title, known) = match (&object, generic_subject) {
                (Some(o), _) => (format!("Deliver {} to {sender}", my_view(o, &sender, true)), true),
                (None, false) => (format!("Deliver to {sender}: {subject}"), true),
                (None, true) => (format!("Meet {sender}'s deadline (confirm what's due)"), false),
            };
            let mut it = base_item(ctx, kind::REQUEST_TO_ME, s);
            it.title = text::truncate(&title, 100);
            it.task_type = "deliverable".into();
            it.expected_deliverable = object.map(|o| cap(&o));
            it.completion_criteria = Some(format!("Delivered to {sender}"));
            it.due_phrase = due.as_ref().map(|d| d.phrase.clone());
            it.due = due;
            it.urgent = CONSEQUENCE.is_match(&l) || it.urgent;
            it.confidence = if multi { 0.55 } else if known { 0.8 } else { 0.7 };
            if !known || multi {
                it.needs_confirmation = true;
                it.ambiguity = Some(if multi {
                    "A deadline was sent to several people — it may not be yours.".into()
                } else {
                    format!("{sender} set a deadline but didn't say what is due — confirm the deliverable.")
                });
            }
            it.description = Some(format!("Deadline set by {sender}."));
            out.push(it);
            continue;
        }
        if let Some(m) = REQUEST.find(&l) {
            if directed && !l.starts_with("thanks") {
                // "can you believe" / "can you imagine" are not requests.
                if Regex::new(r"\b(can|could) you (believe|imagine)\b").unwrap().is_match(&l) {
                    continue;
                }
                let mut conf = if multi { 0.55 } else { 0.85 };
                if ctx.mentions_me(s) {
                    conf = 0.88;
                }
                if let Some(mut it) = task_item(ctx, kind::REQUEST_TO_ME, s, m.end(), true, conf) {
                    // A time named by the requester is their deadline.
                    if let Some(d) = it.due.as_mut() {
                        if !d.before_meeting {
                            d.kind = "explicit".into();
                        }
                    }
                    if m.as_str().contains("someone") || m.as_str().contains("anyone") {
                        it.needs_confirmation = true;
                        it.confidence = it.confidence.min(0.55);
                        it.my_role = "collaborator".into();
                        it.ambiguity = Some("Asked to \"someone\" — not necessarily you.".into());
                    }
                    if it.title.to_lowercase().starts_with("let me know") || it.title.to_lowercase().starts_with("let ") {
                        it.task_type = "reply".into();
                    }
                    if multi {
                        it.needs_confirmation = true;
                        it.ambiguity = Some("Sent to several people without naming you — ownership is unclear.".into());
                        it.my_role = "collaborator".into();
                    }
                    it.description = Some(format!("Requested by {sender}."));
                    out.push(it);
                }
                continue;
            }
        }
        if let Some(m) = FEEDBACK_ASK.find(&l) {
            if directed {
                let due = clause_due(ctx, s, m.end());
                let rest = action_phrase(s, m.end(), due.as_ref().map(|d| d.phrase.as_str()));
                let rest = my_view(&rest, &sender, true);
                let mut it = base_item(ctx, kind::REQUEST_TO_ME, s);
                it.title = text::truncate(&format!("Give {sender} feedback {}", if rest.is_empty() { format!("on {subject}") } else { rest }), 100);
                it.task_type = "review".into();
                it.confidence = if multi { 0.55 } else { 0.75 };
                it.needs_confirmation = multi;
                it.completion_criteria = Some(format!("Feedback sent to {sender}"));
                it.due_phrase = due.as_ref().map(|d| d.phrase.clone());
                it.due = due;
                out.push(it);
                continue;
            }
        }
        if let Some(c) = ON_YOU.captures(s) {
            if directed && !multi && !l.contains('?') {
                let subj = c.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                let tail = c.get(2).map(|m| m.as_str().trim().trim_end_matches(['.', '!'])).unwrap_or("");
                let mut it = base_item(ctx, kind::IMPLICIT_ASSIGNMENT, s);
                let subj_l = subj.to_lowercase();
                let subj_c = if subj_l.starts_with("the ") { subj_l.clone() } else { subj.to_string() };
                it.title = text::truncate(&format!("Handle {}{}", subj_c, if tail.is_empty() { String::new() } else { format!(" {tail}") }), 100);
                it.task_type = if DELIVERABLE.is_match(subj) { "deliverable".into() } else { "action".into() };
                it.confidence = 0.6;
                it.needs_confirmation = true;
                it.ambiguity = Some(format!("{sender} implies this is yours — confirm you own it."));
                it.due = due_for(ctx, tail);
                it.due_phrase = it.due.as_ref().map(|d| d.phrase.clone());
                out.push(it);
                continue;
            }
        }
        if LET_ME_KNOW.is_match(&l) && directed && !multi {
            let mut it = base_item(ctx, kind::REPLY_NEEDED, s);
            let what = action_phrase(s, LET_ME_KNOW.find(&l).unwrap().end(), None);
            let what = my_view(&what, &sender, true);
            it.title = text::truncate(&format!("Reply to {sender}{}", if what.is_empty() { format!(": {subject}") } else { format!(" {what}") }), 100);
            it.task_type = "reply".into();
            it.confidence = 0.65;
            it.completion_criteria = Some(format!("Reply sent to {sender}"));
            it.due = due_for(ctx, s);
            it.due_phrase = it.due.as_ref().map(|d| d.phrase.clone());
            out.push(it);
            continue;
        }
        if let Some(m) = IMPLICIT.find(&l) {
            if directed {
                let verb = m.as_str();
                let lead = if verb.contains("presenting") {
                    "Present"
                } else if verb.contains("preparing") || verb.contains("putting together") {
                    "Prepare"
                } else if verb.contains("drafting") {
                    "Draft"
                } else if verb.contains("sending") {
                    "Send"
                } else if verb.contains("leading") || verb.contains("running") {
                    "Lead"
                } else if verb.contains("covering") {
                    "Cover"
                } else {
                    "Deliver"
                };
                let due = clause_due(ctx, s, m.end());
                let rest = action_phrase(s, m.end(), due.as_ref().map(|d| d.phrase.as_str()));
                let rest = my_view(&rest, &sender, true);
                if rest.split_whitespace().count() >= 1 {
                    let mut it = base_item(ctx, kind::IMPLICIT_ASSIGNMENT, s);
                    it.title = text::truncate(&format!("{lead} {rest}"), 100);
                    it.task_type = infer_type(&it.title, s);
                    if it.task_type == "action" && deliverable_of(&rest).is_some() {
                        it.task_type = "deliverable".into();
                    }
                    it.expected_deliverable = deliverable_of(&rest).map(|d| cap(&d));
                    it.completion_criteria = it.expected_deliverable.as_ref().map(|d| format!("{d} delivered"));
                    it.due_phrase = due.as_ref().map(|d| d.phrase.clone());
                    it.due = due;
                    it.confidence = 0.65;
                    it.needs_confirmation = true;
                    it.ambiguity = Some(format!("Nobody asked you directly — {sender}'s message implies you own this."));
                    out.push(it);
                }
                // fall through: the same sentence may also contain their promise
            }
        }
        // Someone else promising something to me → waiting-for, never my task.
        if let Some(m) = THEIR_COMMIT.find(&l) {
            if (ctx.addressed_to_me() || ctx.mentions_me(s) || ctx.item.kind == "chat") && !LET_ME_KNOW.is_match(&l) && !HYPOTHETICAL_LEAD.is_match(&l[..m.start()]) {
                let due = clause_due(ctx, s, m.end());
                let phrase = action_phrase(s, m.end(), due.as_ref().map(|d| d.phrase.as_str()));
                let phrase = my_view(&phrase, &sender, true);
                if phrase.split_whitespace().count() >= 2 {
                    let mut it = base_item(ctx, kind::THEIR_COMMITMENT, s);
                    it.title = text::truncate(&format!("{sender} to {phrase}"), 100);
                    it.my_role = "requester".into();
                    it.owner_name = ctx.item.sender_name.clone();
                    it.due_phrase = due.as_ref().map(|d| d.phrase.clone());
                    it.due = due;
                    it.confidence = 0.75;
                    out.push(it);
                }
            }
        }
    }

    // A direct question to me alone, with nothing else actionable → reply needed.
    if !from_me
        && out.iter().all(|x| !kind::creates_task(&x.kind))
        && ctx.addressed_to_me()
        && ctx.other_to_count() == 0
        && ctx.item.kind == "email"
    {
        if let Some(q) = sents.iter().rev().find(|s| s.trim_end().ends_with('?')) {
            let l = norm(q);
            let rhetorical = Regex::new(r"\b(how are you|how's it going|how was|how did|how's the|how is the family|did you enjoy|hope you|can you believe|isn't it|right\?$|any plans for)").unwrap().is_match(&l);
            if !rhetorical && q.split_whitespace().count() >= 4 {
                let mut it = base_item(ctx, kind::REPLY_NEEDED, q);
                it.title = text::truncate(&format!("Reply to {sender}: {}", if subject.is_empty() { q.as_str() } else { subject.as_str() }), 100);
                it.task_type = "reply".into();
                it.confidence = 0.6;
                it.needs_confirmation = true;
                it.ambiguity = Some("A question addressed to you — may only need a quick answer.".into());
                it.completion_criteria = Some(format!("Reply sent to {sender}"));
                out.push(it);
            }
        }
    }

    if super::looks_like_injection(&text) {
        for it in out.iter_mut() {
            it.needs_confirmation = true;
            it.confidence = it.confidence.min(0.5);
            it.ambiguity = Some("This message contains text that looks like instructions to an AI assistant (possible prompt injection). Kept treated it as plain text — review before trusting.".into());
        }
    }
    // Keep at most a handful per message; prefer higher confidence.
    out.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(6);
    for it in out.iter_mut() {
        it.confidence = (it.confidence * 100.0).round() / 100.0;
        it.confidence = it.confidence.min(0.97);
    }
    out
}

/// Convenience for tests and the import screen: run rules on a single message.
pub fn extract_one(prefs: &Preferences, item: &SourceItem, thread: &[SourceItem]) -> Vec<Extracted> {
    let ctx = ExtractContext { prefs, item, thread, open_tasks: &[], now: chrono::Utc::now() };
    extract(&ctx)
}
