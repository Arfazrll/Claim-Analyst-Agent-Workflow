# Architecture

## 1. What the system does

A claims analyst submits a structured health-insurance claim. Five specialised agents
decide whether it is admissible under one policy wording (Universal Sompo CSC Individual
Health Insurance), cite the clause behind every statement, verify each citation, and
abstain (`NEEDS_REVIEW`) instead of guessing. A reviewer then signs off or overrides the
decision. Every step is encrypted at rest and sealed into a tamper-evident ledger
anchored to a blockchain.

This is a Rust/Vue re-engineering of
[aptino-claim-engine](https://github.com/ByteNinjaSmit/aptino-claim-engine) (Python,
FastAPI, Streamlit), extended into a multi-user, persistent, deployable product.

## 2. Monorepo layout

```
backend/                 Rust workspace
  crates/engine/         claim engine library (no I/O): agents, rules, retrieval, verification, eval
  crates/api/            axum HTTP API: auth, persistence, cache, rate limits, ledger, chain, IPFS
  data/policy/           policy wording, one clause per chunk (chunks.jsonl)
  data/cases/            29 labelled public / custom / adversarial test cases
frontend/                Vue 3 + TypeScript SPA (Vite, Pinia, vue-router), nginx image
blockchain/              ClaimLedger.sol (Foundry) + deployer image
infra/nginx/             edge reverse proxy / load balancer (compose)
infra/redis/             hardened Redis config
infra/k8s/               kustomize base + dev / prod overlays
scripts/                 secret generation, end-to-end smoke test
.github/workflows/       CI (tests, audits, scans, e2e) and CD (signed images, k8s deploy)
```

## 3. Runtime topology

```
                 ┌──────────── edge nginx / k8s ingress ────────────┐
 browser ──TLS──►│ rate limits · security headers · micro-cache     │
                 └───────┬───────────────────────────┬──────────────┘
                         │ /                         │ /api  (least-conn / EWMA)
                   ┌─────▼─────┐          ┌──────────▼──────────┐
                   │ frontend  │          │ api  x N (stateless)│──► LLM (optional, HTTPS)
                   │ nginx SPA │          └───┬──────┬──────┬───┘
                   └───────────┘              │      │      │
                                  ┌───────────▼┐ ┌───▼───┐ ┌▼────────────────┐
                                  │ PostgreSQL │ │ Redis │ │ anchor worker x1│
                                  │ (SQLite in │ │ cache │ └──┬───────────┬──┘
                                  │  dev)      │ │ limits│    │           │
                                  └────────────┘ └───────┘ ┌──▼───┐  ┌────▼─────────────┐
                                                           │ IPFS │  │ EVM: ClaimLedger │
                                                           └──────┘  └──────────────────┘
```

* **API replicas are stateless.** Sessions are signed cookies; cache, rate-limit
  counters, idempotency keys and the worker lease live in Redis; data lives in
  PostgreSQL. Any replica can serve any request, so the load balancer needs no stickiness.
* **SQLite** is the zero-setup default for local development (`cargo run`). The same
  portable schema runs on PostgreSQL for multi-replica deployments (sqlx `Any` driver).
* **One anchor worker** signs blockchain transactions. A Redis leader lease (compose) or
  a single-replica `Recreate` Deployment (Kubernetes) prevents nonce collisions.

## 4. The agent pipeline

```
Case Analysis ─► Policy Evidence ─► Coverage & Exclusion ─► Decision ─► Validation
                       ▲                                                   │
                       └─────────── retry with widened retrieval (once) ───┘
```

| Agent | Reads | Writes |
|---|---|---|
| Case Analysis | raw case | normalised facts, applicable rules, missing / unknown fields |
| Policy Evidence | facts, rules | ranked evidence per rule (hybrid retrieval) |
| Coverage & Exclusion | facts, evidence | findings with citations + machine-checkable assertions, limits |
| Decision | findings, limits | decision by a precedence rule, confidence, rationale |
| Validation | everything | SUPPORTED / UNSUPPORTED / CONTRADICTED per claim; downgrade on failure |

Agents share one typed `CaseState`; each records what it read and wrote, and every
hand-off (including retries) is kept, which the UI shows as a routing slip.

### Eleven rules

Initial 30-day wait, first-year named-disease wait, pre-existing disease wait, hospital
definition, domiciliary conditions, day care under 24 h, pre/post-hospitalisation
window, cosmetic exclusion, experimental treatment, category sub-limits, and an
*unmodelled policy risk* safety net. Each rule **reads its number from the retrieved
clause** (e.g. "30" from "waiting period of 30 days") and reports insufficient evidence
when the number is not there; nothing is hard-coded from outside knowledge.

### Retrieval (pure Rust, no model download)

`TF-IDF vectors (stems + bigrams) + BM25 → reciprocal rank fusion → lexical cross-scorer`.
Each result carries its rank in every stage. Rules pick the highest-ranked chunk that
actually states what they need (assertion-aware selection), which made retrieval robust
without neural models.

### Improvements over the reference

| Area | Reference | This project |
|---|---|---|
| Paraphrased diagnoses | caught only with an LLM | deterministic clinical-synonym lexicon (can only hold for review) |
| False alarms | "internal" fixation flagged a tumour clause | clauses owned by modelled rules are skipped; anatomical qualifiers ignored |
| Persistence | stateless, reviews downloaded as JSON | encrypted storage, server-side reviews, re-analysis with versioning |
| Accountability | none | accounts, roles, four-eyes review, audit log |
| Integrity | none | hash-chained ledger + IPFS + on-chain anchoring, per-claim proofs |
| Operations | single container | load-balanced replicas, Redis cache, rate limits, K8s, CI/CD |

Evaluation (`cargo run -p claim-engine --bin claim-eval`): 28/28 labelled decisions,
0 unsafe, 10/10 required abstentions, 11/11 exact payable amounts, 139/139 claims
verified, 177/177 injected faults detected, deterministic, ~6 ms per case.

## 5. Ledger and Web3

Health data must not be written to a public blockchain: it is public and immutable, so
it can never be deleted (a privacy and compliance violation). The design therefore keeps
data private and puts only proofs on-chain:

1. Each event (`claim_analyzed`, `claim_reanalyzed`, `claim_reviewed`) produces a small
   document of hashes (case hash, analysis hash, decision, reviewer notes hash).
2. The event is appended to a hash chain in the database:
   `entry_hash = sha256(prev_hash | seq | claim | kind | payload_hash | time)`, inside the
   same transaction as the claim write. Rewriting any past record breaks every later hash.
3. The worker pins the **encrypted** bundle to IPFS (content-addressed CID) and calls
   `ClaimLedger.anchor(recordId, entryHash, kind, cid)` on an EVM chain.
4. `GET /api/claims/{id}/proof` re-derives everything: stored data → hashes → chain links
   → on-chain `isAnchored(entryHash)`.

Chain choice is configuration: local **Anvil** (free, compose/dev), **Sepolia** testnet
via a free public RPC (prod overlay), or any EVM mainnet / L2 later.

## 6. Caching, cookies, rate limiting, load balancing

| Concern | Mechanism |
|---|---|
| Analysis cache | Redis, key = sha256(engine version, LLM setting, canonical case); deterministic so safe |
| Search / stats / eval cache | Redis with short TTLs; stats invalidated on writes |
| HTTP caching | hashed assets `immutable` for a year; `index.html` revalidated; policy text ETag/304 |
| Edge micro-cache | nginx caches policy endpoints per session cookie for 5 min |
| Cookies | `caw_access` (HttpOnly, SameSite=Strict, 15 min JWT), `caw_refresh` (HttpOnly, path `/api/auth`, rotated, reuse revokes family), `caw_csrf` (readable, double-submit) |
| Rate limiting | Redis sliding window per bucket: auth 10/min/IP, analyze 30/min/user, API 300/min/IP; `RateLimit-*` + `Retry-After` headers; nginx/ingress limits in front |
| Load balancing | nginx `least_conn` across replicas with passive health checks and retry; ingress-nginx EWMA in Kubernetes; readiness probes gate traffic |
| Idempotency | `Idempotency-Key` header on claim creation (24 h, replays the same claim) |
