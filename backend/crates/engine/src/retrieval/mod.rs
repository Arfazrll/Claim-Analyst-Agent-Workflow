//! Hybrid retrieval over the policy chunks.
//!
//! `vector (TF-IDF cosine) + BM25 -> Reciprocal Rank Fusion -> lexical cross-scorer rerank`
//!
//! The reference design used a neural embedder and cross-encoder. Here every stage is
//! pure Rust with no model download or GPU: a TF-IDF vector space with stemming and
//! bigrams stands in for the dense stage, and the reranker scores each candidate
//! jointly against the query (IDF-weighted coverage, phrase/bigram matches, length
//! normalised density). The corpus is ~116 chunks, so exact search is used instead of
//! an ANN index. Every result carries per-stage rank and score so retrieval quality
//! can be measured and shown to reviewers.

mod bm25;
mod rerank;
mod text;
mod vector;

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::model::EvidenceItem;

pub use text::{stem, tokenize, tokens_stemmed};

/// One policy chunk as produced by structure-aware chunking of the policy PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyChunk {
    pub chunk_id: String,
    pub section: String,
    pub label: String,
    pub page_start: i64,
    pub page_end: i64,
    pub text: String,
    #[serde(default)]
    pub order: i64,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "policy.pdf".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopK {
    pub dense: usize,
    pub sparse: usize,
    pub fused: usize,
    pub final_k: usize,
}

impl Default for TopK {
    fn default() -> Self {
        TopK {
            dense: 15,
            sparse: 15,
            fused: 10,
            final_k: 5,
        }
    }
}

impl TopK {
    pub fn widened(self, factor: usize) -> Self {
        TopK {
            dense: self.dense * factor,
            sparse: self.sparse * factor,
            fused: self.fused * factor,
            final_k: self.final_k * factor,
        }
    }
}

pub type Ranked = Vec<(String, f64)>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stages {
    pub dense: Ranked,
    pub sparse: Ranked,
    pub fused: Ranked,
    pub reranked: Ranked,
}

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("line {line}: {source}")]
    Parse {
        line: usize,
        source: serde_json::Error,
    },
    #[error("policy index is empty")]
    Empty,
    #[error("duplicate chunk id {0}")]
    Duplicate(String),
}

pub struct HybridRetriever {
    chunks: Vec<PolicyChunk>,
    by_id: HashMap<String, usize>,
    bm25: bm25::Bm25,
    vectors: vector::VectorStore,
    reranker: rerank::Reranker,
    cache: Mutex<HashMap<(String, TopK), Stages>>,
}

impl std::fmt::Debug for HybridRetriever {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridRetriever")
            .field("chunks", &self.chunks.len())
            .finish()
    }
}

pub fn rrf(lists: &[&Ranked], k: f64, top_k: usize) -> Ranked {
    let mut fused: BTreeMap<String, f64> = BTreeMap::new();
    let mut first_seen: Vec<String> = vec![];
    for list in lists {
        for (rank, (id, _)) in list.iter().enumerate() {
            let e = fused.entry(id.clone()).or_insert_with(|| {
                first_seen.push(id.clone());
                0.0
            });
            *e += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut out: Ranked = first_seen
        .into_iter()
        .map(|id| {
            let s = fused[&id];
            (id, s)
        })
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(top_k);
    out
}

impl HybridRetriever {
    pub fn from_jsonl(content: &str) -> Result<Self, IndexError> {
        let mut chunks = vec![];
        for (i, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let chunk: PolicyChunk =
                serde_json::from_str(line).map_err(|source| IndexError::Parse {
                    line: i + 1,
                    source,
                })?;
            chunks.push(chunk);
        }
        Self::new(chunks)
    }

    pub fn new(chunks: Vec<PolicyChunk>) -> Result<Self, IndexError> {
        if chunks.is_empty() {
            return Err(IndexError::Empty);
        }
        let mut by_id = HashMap::new();
        for (i, c) in chunks.iter().enumerate() {
            if by_id.insert(c.chunk_id.clone(), i).is_some() {
                return Err(IndexError::Duplicate(c.chunk_id.clone()));
            }
        }
        let docs: Vec<Vec<String>> = chunks.iter().map(|c| tokens_stemmed(&c.text)).collect();
        let bm25 = bm25::Bm25::new(&docs);
        let vectors = vector::VectorStore::new(&docs);
        let reranker = rerank::Reranker::new(&docs, &bm25);
        Ok(HybridRetriever {
            chunks,
            by_id,
            bm25,
            vectors,
            reranker,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn chunks(&self) -> &[PolicyChunk] {
        &self.chunks
    }

    pub fn chunk(&self, id: &str) -> Option<&PolicyChunk> {
        self.by_id.get(id).map(|&i| &self.chunks[i])
    }

    pub fn corpus(&self) -> BTreeMap<String, String> {
        self.chunks
            .iter()
            .map(|c| (c.chunk_id.clone(), c.text.clone()))
            .collect()
    }

    /// Ranked (chunk_id, score) lists after every stage (exposed for ablation).
    pub fn search_stages(&self, query: &str, k: TopK) -> Stages {
        let key = (query.to_string(), k);
        if let Some(hit) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return hit;
        }
        let q = tokens_stemmed(query);
        let id = |i: usize| self.chunks[i].chunk_id.clone();
        let dense: Ranked = self
            .vectors
            .search(&q, k.dense)
            .into_iter()
            .map(|(i, s)| (id(i), s))
            .collect();
        let sparse: Ranked = self
            .bm25
            .search(&q, k.sparse)
            .into_iter()
            .map(|(i, s)| (id(i), s))
            .collect();
        let fused = rrf(&[&dense, &sparse], 60.0, k.fused);
        let candidates: Vec<usize> = fused.iter().map(|(cid, _)| self.by_id[cid]).collect();
        let mut reranked: Ranked = self
            .reranker
            .score(&q, query, &candidates, &self.chunks)
            .into_iter()
            .map(|(i, s)| (id(i), s))
            .collect();
        reranked.truncate(k.final_k);
        let stages = Stages {
            dense,
            sparse,
            fused,
            reranked,
        };
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() < 4096 {
                cache.insert(key, stages.clone());
            }
        }
        stages
    }

    pub fn search(&self, query: &str, k: TopK) -> Vec<EvidenceItem> {
        let st = self.search_stages(query, k);
        let rank_of =
            |list: &Ranked, cid: &str| list.iter().position(|(c, _)| c == cid).map(|p| p + 1);
        let score_of =
            |list: &Ranked, cid: &str| list.iter().find(|(c, _)| c == cid).map(|(_, s)| round4(*s));
        st.reranked
            .iter()
            .map(|(cid, score)| {
                let c = self.chunk(cid).expect("ranked id exists");
                EvidenceItem {
                    chunk_id: c.chunk_id.clone(),
                    section: c.section.clone(),
                    label: c.label.clone(),
                    page_start: c.page_start,
                    page_end: c.page_end,
                    text: c.text.clone(),
                    dense_rank: rank_of(&st.dense, cid),
                    dense_score: score_of(&st.dense, cid),
                    sparse_rank: rank_of(&st.sparse, cid),
                    sparse_score: score_of(&st.sparse, cid),
                    fused_score: score_of(&st.fused, cid),
                    rerank_score: Some(round4(*score)),
                }
            })
            .collect()
    }

    pub fn evidence_for(&self, chunk: &PolicyChunk) -> EvidenceItem {
        EvidenceItem {
            chunk_id: chunk.chunk_id.clone(),
            section: chunk.section.clone(),
            label: chunk.label.clone(),
            page_start: chunk.page_start,
            page_end: chunk.page_end,
            text: chunk.text.clone(),
            dense_rank: None,
            dense_score: None,
            sparse_rank: None,
            sparse_score: None,
            fused_score: None,
            rerank_score: None,
        }
    }

    pub fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
    }
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_prefers_agreement() {
        let a: Ranked = vec![("x".into(), 1.0), ("y".into(), 0.5)];
        let b: Ranked = vec![("y".into(), 9.0), ("z".into(), 1.0)];
        let fused = rrf(&[&a, &b], 60.0, 10);
        assert_eq!(fused[0].0, "y");
        assert_eq!(fused.len(), 3);
    }
}
