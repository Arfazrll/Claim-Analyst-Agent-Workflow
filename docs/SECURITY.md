# Security

## Data protection

* **Encryption at rest:** claim payloads, analyses, review notes and ledger documents are
  AES-256-GCM encrypted with a per-record associated-data binding (a ciphertext moved to
  another record fails to decrypt). Key: `DATA_ENCRYPTION_KEY` (32 random bytes).
* **No personal data on-chain:** only 32-byte hashes and IPFS CIDs of encrypted bundles.
* **Integrity:** hash-chained ledger, on-chain anchors, per-claim proofs, whole-chain
  verification (`GET /api/ledger/verify`).

## Authentication and sessions

* Argon2id password hashing (OWASP parameters), 12+ characters with three character
  classes, must not contain the email name.
* Timing-equalised login (unknown emails still verify a dummy hash) and one generic
  error message, so accounts cannot be enumerated through login.
* Lockout: 5 failed attempts lock the account for 15 minutes (HTTP 423).
* 15-minute access JWT (HS256, issuer-checked) in an HttpOnly, SameSite=Strict cookie.
* Refresh tokens: random 256-bit, stored only as SHA-256, rotated on every use; replay of
  a rotated token outside a 30 s race window revokes the whole session family.
* Password change and account suspension revoke all refresh tokens; suspension and role
  changes apply within 30 s even for live access tokens.
* Bootstrap admin from env; self-registration can be disabled (`ALLOW_REGISTRATION`).

## Authorisation

* Roles: analyst < reviewer < admin. Analysts see only their own claims (other ids return
  404, not 403, to prevent id probing).
* Four-eyes principle: a reviewer cannot sign off a claim they submitted.
* Overrides and escalations require a written reason.

## Web protections

* CSRF: SameSite=Strict cookies + double-submit token bound inside the signed JWT +
  Origin allow-list check on unsafe methods.
* Strict CSP on the SPA (no inline scripts, no third-party origins; fonts self-hosted),
  `X-Frame-Options: DENY`, `nosniff`, HSTS, `Referrer-Policy`, `Permissions-Policy`,
  COOP/CORP. The API itself answers with `default-src 'none'`.
* Untrusted claim text is rendered as text only (`vue/no-v-html` is a lint error) and
  sanitised before it reaches findings or LLM prompts.
* Request body limit (256 KB), request timeout (30 s), input validation with field
  errors, internal errors never leak details.
* Client IP for rate limiting comes from `X-Real-IP` only when `TRUST_PROXY=true` (set by
  the edge from the TCP peer).

## LLM safety

The language model never decides. Rationale text is rejected if it adds any number not in
the evidence, omits the decision, or argues against a non-approving decision. The
optional interpretation step can only push a claim to `NEEDS_REVIEW`; a hostile-model fuzz
test over all 29 cases proves no outcome becomes more permissive.

## Infrastructure

* Distroless, non-root API image; unprivileged nginx; read-only root filesystems;
  `cap_drop: ALL`; `no-new-privileges`.
* Kubernetes: restricted Pod Security Standard, default-deny NetworkPolicies, no service
  account tokens, PodDisruptionBudgets.
* Redis password-protected, dangerous commands disabled, on an internal network.
* Secrets never committed (`.env`, `secrets.env` git-ignored; gitleaks in CI).
* Supply chain: `cargo audit`, `npm audit`, Trivy image scan, Dependabot, keyless cosign
  signatures, SBOM and provenance on pushed images.

## Smart contract

`ClaimLedger` is append-only (no edit or delete), writer-gated, pausable, uses two-step
ownership transfer, rejects zero and duplicate hashes, bounds CID length and batch size,
and uses custom errors. Foundry unit and fuzz tests run in CI.

## Reporting

Report vulnerabilities privately to the repository owner rather than opening an issue.
