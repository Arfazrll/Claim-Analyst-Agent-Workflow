//! Lexical cross-scorer: scores each fused candidate jointly against the query.
//!
//! score = 0.55 * IDF-weighted query coverage
//!       + 0.25 * share of query bigrams found as adjacent stems in the chunk
//!       + 0.20 * length-normalised match density (relative to the best candidate)
//!
//! Unlike BM25 alone, coverage rewards chunks that answer *all* of a query's
//! aspects, and the bigram term rewards phrase-level matches ("waiting period",
//! "sum insured") over scattered single words.

use std::collections::{HashMap, HashSet};

use super::bm25::Bm25;
use super::PolicyChunk;

pub struct Reranker {
    docs: Vec<Vec<String>>,
    idf: HashMap<String, f64>,
    floor: f64,
}

impl Reranker {
    pub fn new(docs: &[Vec<String>], bm25: &Bm25) -> Self {
        let mut idf = HashMap::new();
        for d in docs {
            for t in d {
                idf.entry(t.clone()).or_insert_with(|| bm25.idf(t));
            }
        }
        Reranker {
            docs: docs.to_vec(),
            idf,
            floor: (bm25.max_idf() * 0.05).max(0.05),
        }
    }

    fn weight(&self, t: &str) -> f64 {
        self.idf
            .get(t)
            .copied()
            .unwrap_or(self.floor)
            .max(self.floor)
    }

    pub fn score(
        &self,
        query: &[String],
        _raw_query: &str,
        candidates: &[usize],
        _chunks: &[PolicyChunk],
    ) -> Vec<(usize, f64)> {
        let qset: HashSet<&String> = query.iter().collect();
        let total_w: f64 = qset.iter().map(|t| self.weight(t)).sum::<f64>().max(1e-9);
        let qbigrams: HashSet<(&String, &String)> =
            query.windows(2).map(|w| (&w[0], &w[1])).collect();

        let raw: Vec<(usize, f64, f64, f64)> = candidates
            .iter()
            .map(|&i| {
                let doc = &self.docs[i];
                let dset: HashSet<&String> = doc.iter().collect();
                let covered: f64 = qset
                    .iter()
                    .filter(|t| dset.contains(*t))
                    .map(|t| self.weight(t))
                    .sum();
                let coverage = covered / total_w;
                let dbigrams: HashSet<(&String, &String)> =
                    doc.windows(2).map(|w| (&w[0], &w[1])).collect();
                let bigram = if qbigrams.is_empty() {
                    0.0
                } else {
                    qbigrams.iter().filter(|b| dbigrams.contains(*b)).count() as f64
                        / qbigrams.len() as f64
                };
                let hits: f64 = doc
                    .iter()
                    .filter(|t| qset.contains(t))
                    .map(|t| self.weight(t))
                    .sum();
                let density = hits / (doc.len().max(1) as f64).sqrt();
                (i, coverage, bigram, density)
            })
            .collect();
        let max_density = raw.iter().map(|r| r.3).fold(0.0, f64::max).max(1e-9);
        let mut scored: Vec<(usize, usize, f64)> = raw
            .into_iter()
            .enumerate()
            .map(|(order, (i, c, b, d))| (order, i, 0.55 * c + 0.25 * b + 0.20 * (d / max_density)))
            .collect();
        scored.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        scored.into_iter().map(|(_, i, s)| (i, s)).collect()
    }
}
