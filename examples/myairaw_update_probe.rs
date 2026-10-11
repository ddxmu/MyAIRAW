//! Release QA: run inside a disposable, ad-hoc signed copy of the previous
//! bundle, never the user's installed app. RAWMAKASE_PRETEND_VERSION chooses
//! the base version; --install also rehearses the normal handoff/acknowledgment.
use anyhow::{Context, Result};

fn main() -> Result<()> {
    let launch = rawmakase::updates::intercept();
    let updater = rawmakase::updates::updater();
    let release = updater.check()?.context("No newer release to test")?;
    let prepared = updater.download(&release, |received, total| {
        if received == total {
            println!("Downloaded {received}/{total} bytes");
        }
    })?;
    println!(
        "Verified update {} for {:?}",
        prepared.version(),
        prepared.installation().executable
    );
    if launch.arguments.iter().any(|arg| arg == "--install") {
        updater.handoff(prepared, Vec::new())?;
    } else {
        prepared.discard();
    }
    Ok(())
}
