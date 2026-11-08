//! Tamper-evident claim ledger with Web3 anchoring.
//!
//! ```text
//! claim event --> payload document (hashes only) --> hash-chained ledger entry (DB)
//!                                                      |
//!                          encrypted bundle --> IPFS (CID)      entry hash + CID --> ClaimLedger.sol (EVM)
//! ```
//!
//! * Every claim event (analysed, re-analysed, reviewed) appends an entry whose hash
//!   covers the previous entry's hash, so rewriting any past record breaks the chain.
//! * The payload document holds only hashes and the decision; it is stored encrypted.
//! * A background worker pins the encrypted bundle to IPFS and anchors the entry hash
//!   on-chain. Personal health data never reaches the chain: only 32-byte hashes do.

pub mod chain;
pub mod ipfs;
pub mod worker;

use serde::Serialize;
use serde_json::{json, Value};

use crate::crypto::{sha256_hex, sha256_json, Cipher};
use crate::db::{now_ms, ClaimRow, Db, LedgerEntry};
use crate::error::ApiResult;

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    ClaimAnalyzed,
    ClaimReanalyzed,
    ClaimReviewed,
}

impl EntryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryKind::ClaimAnalyzed => "claim_analyzed",
            EntryKind::ClaimReanalyzed => "claim_reanalyzed",
            EntryKind::ClaimReviewed => "claim_reviewed",
        }
    }
    pub fn code(kind: &str) -> u8 {
        match kind {
            "claim_analyzed" => 1,
            "claim_reanalyzed" => 2,
            "claim_reviewed" => 3,
            _ => 0,
        }
    }
}

/// `sha256(prev | seq | claim_id | kind | payload_hash | created_at)`
pub fn entry_hash(
    prev: &str,
    seq: i64,
    claim_id: &str,
    kind: &str,
    payload_hash: &str,
    created_at: i64,
) -> String {
    sha256_hex(format!("{prev}|{seq}|{claim_id}|{kind}|{payload_hash}|{created_at}").as_bytes())
}

/// bytes32 record id used on-chain for a claim.
pub fn record_id(claim_id: &str) -> String {
    sha256_hex(format!("claim:{claim_id}").as_bytes())
}

fn aad(claim_id: &str) -> String {
    format!("ledger:{claim_id}")
}

#[derive(Clone)]
pub struct Ledger {
    pub db: Db,
    pub cipher: Cipher,
    pub chain: Option<chain::ChainClient>,
    pub ipfs: Option<ipfs::IpfsClient>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

fn check(name: impl Into<String>, passed: bool, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        passed,
        detail: detail.into(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainVerification {
    pub ok: bool,
    pub checked: i64,
    pub head_seq: i64,
    pub head_hash: String,
    pub first_broken_seq: Option<i64>,
    pub detail: String,
}

impl Ledger {
    pub fn initial_status(&self) -> &'static str {
        if self.chain.is_some() || self.ipfs.is_some() {
            "pending"
        } else {
            "local"
        }
    }

    /// Append an event inside the caller's transaction (atomic with the claim write).
    pub async fn append(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        claim_id: &str,
        kind: EntryKind,
        document: Value,
        actor_id: &str,
    ) -> ApiResult<LedgerEntry> {
        let (seq, prev) = self.db.ledger_reserve(tx).await?;
        let created_at = now_ms();
        let payload_hash = sha256_json(&document);
        let hash = entry_hash(
            &prev,
            seq,
            claim_id,
            kind.as_str(),
            &payload_hash,
            created_at,
        );
        let entry = LedgerEntry {
            seq,
            claim_id: claim_id.into(),
            kind: kind.as_str().into(),
            payload_hash,
            prev_hash: prev,
            entry_hash: hash,
            actor_id: actor_id.into(),
            payload_enc: self.cipher.encrypt_json(&document, &aad(claim_id)),
            cid: None,
            chain_status: self.initial_status().into(),
            tx_hash: None,
            block_number: None,
            attempts: 0,
            last_error: None,
            created_at,
            anchored_at: None,
        };
        self.db.ledger_insert(tx, &entry).await?;
        Ok(entry)
    }

    pub fn decrypt_document(&self, e: &LedgerEntry) -> Option<Value> {
        self.cipher
            .decrypt_json(&e.payload_enc, &aad(&e.claim_id))
            .ok()
    }

    /// Bundle pinned to IPFS: metadata in clear, the document encrypted.
    pub fn ipfs_bundle(&self, e: &LedgerEntry) -> Value {
        json!({
            "format": "caw-ledger-bundle/v1",
            "seq": e.seq, "claim_record_id": record_id(&e.claim_id), "kind": e.kind,
            "payload_hash": e.payload_hash, "prev_hash": e.prev_hash, "entry_hash": e.entry_hash,
            "created_at": e.created_at, "ciphertext": e.payload_enc,
        })
    }

    /// Walk the whole chain and recompute every link.
    pub async fn verify_chain(&self) -> ApiResult<ChainVerification> {
        let (head_seq, head_hash) = self.db.ledger_head().await?;
        let mut prev = GENESIS.to_string();
        let mut last = 0i64;
        let mut checked = 0i64;
        loop {
            let batch = self.db.ledger_range(last, 500).await?;
            if batch.is_empty() {
                break;
            }
            for e in &batch {
                checked += 1;
                let recomputed = entry_hash(
                    &e.prev_hash,
                    e.seq,
                    &e.claim_id,
                    &e.kind,
                    &e.payload_hash,
                    e.created_at,
                );
                let doc_ok = self
                    .decrypt_document(e)
                    .map(|d| sha256_json(&d) == e.payload_hash)
                    .unwrap_or(false);
                let problem = if e.seq != last + 1 {
                    Some(format!("sequence gap before #{}", e.seq))
                } else if e.prev_hash != prev {
                    Some(format!(
                        "entry #{} does not link to the previous entry",
                        e.seq
                    ))
                } else if recomputed != e.entry_hash {
                    Some(format!("entry #{} hash does not match its contents", e.seq))
                } else if !doc_ok {
                    Some(format!(
                        "entry #{} payload document does not match its hash",
                        e.seq
                    ))
                } else {
                    None
                };
                if let Some(detail) = problem {
                    return Ok(ChainVerification {
                        ok: false,
                        checked,
                        head_seq,
                        head_hash,
                        first_broken_seq: Some(e.seq),
                        detail,
                    });
                }
                prev = e.entry_hash.clone();
                last = e.seq;
            }
        }
        let head_ok = last == head_seq && (head_seq == 0 || prev == head_hash);
        Ok(ChainVerification {
            ok: head_ok,
            checked,
            head_seq,
            head_hash: head_hash.clone(),
            first_broken_seq: if head_ok { None } else { Some(last + 1) },
            detail: if head_ok {
                format!("{checked} entries verified; head matches")
            } else {
                "ledger head does not match the last entry".into()
            },
        })
    }

    /// End-to-end integrity proof for one claim.
    pub async fn prove_claim(
        &self,
        claim: &ClaimRow,
        case: Option<&Value>,
        analysis: Option<&Value>,
    ) -> ApiResult<Vec<Check>> {
        let mut checks = vec![];
        match case {
            Some(c) => {
                let h = sha256_json(c);
                checks.push(check(
                    "stored claim payload matches its recorded hash",
                    h == claim.payload_hash,
                    format!("sha256 {}", &h[..16]),
                ));
            }
            None => checks.push(check(
                "stored claim payload decrypts",
                false,
                "decryption failed (wrong key or tampered data)",
            )),
        }
        match analysis {
            Some(a) => {
                let h = sha256_json(a);
                checks.push(check(
                    "stored analysis matches its recorded hash",
                    h == claim.analysis_hash,
                    format!("sha256 {}", &h[..16]),
                ));
            }
            None => checks.push(check(
                "stored analysis decrypts",
                false,
                "decryption failed (wrong key or tampered data)",
            )),
        }
        let entries = self.db.ledger_for_claim(&claim.id).await?;
        checks.push(check(
            "claim has ledger entries",
            !entries.is_empty(),
            format!("{} entr(y/ies)", entries.len()),
        ));
        for e in &entries {
            let recomputed = entry_hash(
                &e.prev_hash,
                e.seq,
                &e.claim_id,
                &e.kind,
                &e.payload_hash,
                e.created_at,
            );
            checks.push(check(
                format!("entry #{} hash recomputes", e.seq),
                recomputed == e.entry_hash,
                e.entry_hash[..16].to_string(),
            ));
            let prev_ok = if e.seq == 1 {
                e.prev_hash == GENESIS
            } else {
                self.db
                    .ledger_by_seq(e.seq - 1)
                    .await?
                    .map(|p| p.entry_hash == e.prev_hash)
                    .unwrap_or(false)
            };
            checks.push(check(
                format!("entry #{} links to entry #{}", e.seq, e.seq - 1),
                prev_ok,
                "",
            ));
            let doc = self.decrypt_document(e);
            let doc_ok = doc
                .as_ref()
                .map(|d| sha256_json(d) == e.payload_hash)
                .unwrap_or(false);
            checks.push(check(
                format!("entry #{} payload document matches", e.seq),
                doc_ok,
                e.kind.clone(),
            ));
            if let (Some(chain), Some(_)) = (&self.chain, &e.tx_hash) {
                match chain.is_anchored(&e.entry_hash).await {
                    Ok(true) => checks.push(check(
                        format!("entry #{} is anchored on-chain", e.seq),
                        true,
                        e.tx_hash.clone().unwrap_or_default(),
                    )),
                    Ok(false) => checks.push(check(
                        format!("entry #{} is anchored on-chain", e.seq),
                        false,
                        "contract does not know this hash",
                    )),
                    Err(err) => checks.push(check(
                        format!("entry #{} is anchored on-chain", e.seq),
                        false,
                        format!("chain unreachable: {err}"),
                    )),
                }
            }
        }
        // The latest analysis event must commit to the analysis currently stored.
        if let Some(last) = entries.iter().rev().find(|e| e.kind != "claim_reviewed") {
            let committed = self.decrypt_document(last).and_then(|d| {
                d.get("analysis_hash")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            checks.push(check(
                "latest ledger entry commits to the current analysis",
                committed.as_deref() == Some(claim.analysis_hash.as_str()),
                committed
                    .map(|c| c[..16.min(c.len())].to_string())
                    .unwrap_or_default(),
            ));
        }
        Ok(checks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_hash_depends_on_every_field() {
        let base = entry_hash(GENESIS, 1, "c", "claim_analyzed", "p", 10);
        assert_ne!(base, entry_hash(GENESIS, 2, "c", "claim_analyzed", "p", 10));
        assert_ne!(base, entry_hash(GENESIS, 1, "d", "claim_analyzed", "p", 10));
        assert_ne!(base, entry_hash(GENESIS, 1, "c", "claim_reviewed", "p", 10));
        assert_ne!(base, entry_hash(GENESIS, 1, "c", "claim_analyzed", "q", 10));
        assert_ne!(base, entry_hash(GENESIS, 1, "c", "claim_analyzed", "p", 11));
        assert_eq!(base.len(), 64);
    }
}
