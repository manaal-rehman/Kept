//! Developer helper: create a Kept database pre-loaded with the sample workspace.
//! Usage: cargo run -p kept-core --example seed -- <data-dir> [timezone]
use kept_core::{secrets::MemoryStore, Agent};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).expect("data dir");
    let tz = std::env::args().nth(2).unwrap_or_else(|| "UTC".into());
    let a = Agent::open(std::path::Path::new(&dir), Arc::new(MemoryStore::default()))?;
    let mut p = a.prefs()?;
    p.onboarded = true;
    p.user_name = "Alex Morgan".into();
    p.user_emails = vec!["alex@example.com".into()];
    p.timezone = tz;
    p.quiet_hours_enabled = false;
    a.save_prefs(&p)?;
    println!("{}", a.load_sample()?);
    let (n, s) = a.process_pending(500).await?;
    println!("processed {n}: {s:?}");
    let r = a.tick().await;
    println!("tick: {} notifications, {} toasts", r.notifications_created, r.toasts.len());
    Ok(())
}
