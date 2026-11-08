//! Outbox worker: pins pending ledger entries to IPFS and anchors them on-chain.
//!
//! Several API replicas may run the worker. A leader lease in the shared cache keeps a
//! single active anchorer (one signer key means one nonce sequence), and each entry is
//! additionally leased in the database so a crashed worker's entry is retried later.
//! Failures back off exponentially and give up after `MAX_ATTEMPTS` (re-queueable by
//! an admin).

use std::time::Duration;

use super::{record_id, EntryKind, Ledger};
use crate::cache::Cache;

const MAX_ATTEMPTS: i64 = 8;
const LEADER_KEY: &str = "anchor:leader";

pub async fn run(ledger: Ledger, cache: Cache, interval: Duration, instance: String) {
    if ledger.chain.is_none() && ledger.ipfs.is_none() {
        tracing::info!("anchor worker idle: neither chain nor IPFS is enabled (ledger is local hash-chain only)");
        return;
    }
    tracing::info!(instance, "anchor worker started");
    let lease = Duration::from_secs((interval.as_secs() * 4).max(15));
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let leader = cache.get(LEADER_KEY).await;
        let is_leader = match leader.as_deref() {
            Some(id) if id == instance => {
                cache.set(LEADER_KEY, &instance, lease).await;
                true
            }
            Some(_) => false,
            None => cache.set_nx(LEADER_KEY, &instance, lease).await,
        };
        if !is_leader {
            continue;
        }
        if let Err(e) = tick(&ledger).await {
            tracing::warn!(error = %e, "anchor tick failed");
        }
    }
}

pub async fn tick(ledger: &Ledger) -> Result<usize, String> {
    let pending = ledger
        .db
        .ledger_pending(20)
        .await
        .map_err(|e| e.to_string())?;
    let mut done = 0;
    for entry in pending {
        if !ledger
            .db
            .ledger_lease(entry.seq, 120_000)
            .await
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        let mut cid = entry.cid.clone();
        let result: Result<(Option<String>, Option<u64>), String> = async {
            if let (Some(ipfs), None) = (&ledger.ipfs, &cid) {
                let c = ipfs
                    .add_json(
                        &format!("ledger-{}", entry.seq),
                        &ledger.ipfs_bundle(&entry),
                    )
                    .await?;
                ledger
                    .db
                    .ledger_set_cid(entry.seq, &c)
                    .await
                    .map_err(|e| e.to_string())?;
                cid = Some(c);
            }
            match &ledger.chain {
                Some(chain) => {
                    let (tx, block) = chain
                        .anchor(
                            &record_id(&entry.claim_id),
                            &entry.entry_hash,
                            EntryKind::code(&entry.kind),
                            cid.as_deref().unwrap_or(""),
                        )
                        .await?;
                    Ok((Some(tx), block))
                }
                None => Ok((None, None)),
            }
        }
        .await;
        match result {
            Ok((tx, block)) => {
                let status = if tx.is_some() { "anchored" } else { "pinned" };
                ledger
                    .db
                    .ledger_mark(entry.seq, status, tx.as_deref(), block.map(|b| b as i64))
                    .await
                    .map_err(|e| e.to_string())?;
                tracing::info!(
                    seq = entry.seq,
                    status,
                    tx = tx.as_deref().unwrap_or("-"),
                    cid = cid.as_deref().unwrap_or("-"),
                    "ledger entry anchored"
                );
                done += 1;
            }
            Err(err) => {
                let backoff = (2i64.pow(entry.attempts.min(10) as u32) * 5_000).min(30 * 60_000);
                tracing::warn!(seq = entry.seq, attempt = entry.attempts + 1, error = %err, "anchoring failed; will retry");
                ledger
                    .db
                    .ledger_fail(entry.seq, &err, backoff, MAX_ATTEMPTS)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(done)
}
