//! The import pipeline: pull items from a [`Source`], upload them with the
//! [`Uploader`], and record progress in the [`Journal`].

use crate::ente::{UploadOutcome, Uploader};
use crate::item::Source;
use crate::state::Journal;

/// Totals after a run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Newly uploaded files.
    pub uploaded: u64,
    /// Items whose content already existed in the library.
    pub duplicates: u64,
    /// Items skipped because the journal says they were done.
    pub skipped: u64,
    /// Items that failed (logged; the run continues).
    pub failed: u64,
    /// Items prepared in a dry run.
    pub dry_run: u64,
}

/// Import everything `source` yields. Failures of single items are logged
/// and counted; the run only aborts on source errors.
pub async fn run(
    source: &mut dyn Source,
    uploader: &mut Uploader,
    journal: &mut Journal,
) -> anyhow::Result<Summary> {
    let mut s = Summary::default();
    let total = source.total().await?;
    if let Some(t) = total {
        tracing::info!(source = source.name(), total = t, already_done = journal.len(), "starting import");
    }
    loop {
        let skipped = std::sync::atomic::AtomicU64::new(0);
        let item = {
            let j = &*journal;
            let skip = |id: &str| {
                let hit = j.contains(id);
                if hit {
                    skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                hit
            };
            source.next(&skip).await?
        };
        s.skipped += skipped.into_inner();
        let Some(item) = item else { break };
        match uploader.import(&item).await {
            Ok(UploadOutcome::Uploaded(id)) => {
                s.uploaded += 1;
                journal.record(&item.source_id, id)?;
            }
            Ok(UploadOutcome::Duplicate(id)) => {
                s.duplicates += 1;
                journal.record(&item.source_id, id)?;
            }
            Ok(UploadOutcome::DryRun) => s.dry_run += 1,
            Err(e) => {
                s.failed += 1;
                tracing::error!(source_id = %item.source_id, file = %item.primary.file_name, error = %format!("{e:#}"), "item failed");
            }
        }
        source.done(&item).await?;
        let n = s.uploaded + s.duplicates + s.failed + s.dry_run;
        if n % 50 == 0 {
            tracing::info!(processed = n, total = ?total, "progress");
        }
    }
    tracing::info!(?s, "import finished");
    Ok(s)
}
