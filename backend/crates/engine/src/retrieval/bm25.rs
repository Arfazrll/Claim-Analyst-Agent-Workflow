//! BM25 Okapi lexical retrieval (k1 = 1.5, b = 0.75, epsilon floor for negative IDF,
//! matching the `rank_bm25` defaults used by the reference implementation).

use std::collections::HashMap;

pub struct Bm25 {
    doc_freqs: Vec<HashMap<String, f64>>,
    doc_len: Vec<f64>,
    avgdl: f64,
    idf: HashMap<String, f64>,
    k1: f64,
    b: f64,
}

impl Bm25 {
    pub fn new(docs: &[Vec<String>]) -> Self {
        let n = docs.len() as f64;
        let mut df: HashMap<String, f64> = HashMap::new();
        let mut doc_freqs = vec![];
        let mut doc_len = vec![];
        for d in docs {
            let mut tf: HashMap<String, f64> = HashMap::new();
            for t in d {
                *tf.entry(t.clone()).or_default() += 1.0;
            }
            for t in tf.keys() {
                *df.entry(t.clone()).or_default() += 1.0;
            }
            doc_len.push(d.len() as f64);
            doc_freqs.push(tf);
        }
        let avgdl = doc_len.iter().sum::<f64>() / n.max(1.0);
        let mut idf: HashMap<String, f64> = HashMap::new();
        let mut negatives = vec![];
        let mut sum = 0.0;
        for (t, f) in &df {
            let v = ((n - f + 0.5) / (f + 0.5)).ln();
            sum += v;
            if v < 0.0 {
                negatives.push(t.clone());
            }
            idf.insert(t.clone(), v);
        }
        let eps = 0.25 * sum / (idf.len().max(1) as f64);
        for t in negatives {
            idf.insert(t, eps);
        }
        Bm25 {
            doc_freqs,
            doc_len,
            avgdl,
            idf,
            k1: 1.5,
            b: 0.75,
        }
    }

    pub fn idf(&self, term: &str) -> f64 {
        self.idf.get(term).copied().unwrap_or(0.0)
    }

    /// Max IDF in the corpus (used to weight unseen query terms in the reranker).
    pub fn max_idf(&self) -> f64 {
        self.idf.values().copied().fold(0.0, f64::max)
    }

    pub fn scores(&self, query: &[String]) -> Vec<f64> {
        let mut scores = vec![0.0; self.doc_freqs.len()];
        for q in query {
            let idf = self.idf(q);
            if idf == 0.0 {
                continue;
            }
            for (i, tf) in self.doc_freqs.iter().enumerate() {
                if let Some(&f) = tf.get(q) {
                    let denom =
                        f + self.k1 * (1.0 - self.b + self.b * self.doc_len[i] / self.avgdl);
                    scores[i] += idf * (f * (self.k1 + 1.0)) / denom;
                }
            }
        }
        scores
    }

    pub fn search(&self, query: &[String], top_k: usize) -> Vec<(usize, f64)> {
        let scores = self.scores(query);
        let mut ranked: Vec<(usize, f64)> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, s)| *s > 0.0)
            .collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        ranked.truncate(top_k);
        ranked
    }
}
