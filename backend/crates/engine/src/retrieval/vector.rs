//! TF-IDF vector space (unigram stems + adjacent bigrams, sublinear TF, L2-normalised),
//! searched by exact cosine similarity. Plays the "dense" role in the hybrid pipeline
//! without any model download.

use std::collections::HashMap;

pub struct VectorStore {
    vocab_idf: HashMap<String, f64>,
    docs: Vec<HashMap<String, f64>>,
}

fn features(tokens: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tokens.to_vec();
    out.extend(tokens.windows(2).map(|w| format!("{} {}", w[0], w[1])));
    out
}

fn tf_vector(feats: &[String], idf: &HashMap<String, f64>) -> HashMap<String, f64> {
    let mut tf: HashMap<String, f64> = HashMap::new();
    for f in feats {
        *tf.entry(f.clone()).or_default() += 1.0;
    }
    let mut v: HashMap<String, f64> = tf
        .into_iter()
        .filter_map(|(f, c)| idf.get(&f).map(|w| (f, (1.0 + c.ln()) * w)))
        .collect();
    let norm = v.values().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        v.values_mut().for_each(|x| *x /= norm);
    }
    v
}

impl VectorStore {
    pub fn new(docs: &[Vec<String>]) -> Self {
        let n = docs.len() as f64;
        let mut df: HashMap<String, f64> = HashMap::new();
        let feats: Vec<Vec<String>> = docs.iter().map(|d| features(d)).collect();
        for f in &feats {
            let mut uniq: Vec<&String> = f.iter().collect();
            uniq.sort();
            uniq.dedup();
            for t in uniq {
                *df.entry(t.clone()).or_default() += 1.0;
            }
        }
        let vocab_idf: HashMap<String, f64> = df
            .into_iter()
            .map(|(t, d)| (t, ((1.0 + n) / (1.0 + d)).ln() + 1.0))
            .collect();
        let docs = feats.iter().map(|f| tf_vector(f, &vocab_idf)).collect();
        VectorStore { vocab_idf, docs }
    }

    pub fn search(&self, query: &[String], top_k: usize) -> Vec<(usize, f64)> {
        let q = tf_vector(&features(query), &self.vocab_idf);
        if q.is_empty() {
            return vec![];
        }
        let mut ranked: Vec<(usize, f64)> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                (
                    i,
                    q.iter()
                        .map(|(t, w)| w * d.get(t).copied().unwrap_or(0.0))
                        .sum::<f64>(),
                )
            })
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
