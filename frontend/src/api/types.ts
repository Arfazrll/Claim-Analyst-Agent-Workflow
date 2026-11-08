export type Decision =
  | 'ADMISSIBLE'
  | 'ADMISSIBLE_WITH_LIMITS'
  | 'PARTIALLY_ADMISSIBLE'
  | 'NOT_ADMISSIBLE'
  | 'NEEDS_REVIEW'

export type Role = 'analyst' | 'reviewer' | 'admin'

export interface User {
  id: string
  email: string
  name: string
  role: Role
  active: boolean
  created_at: number
}

export interface ClaimSummary {
  id: string
  case_id: string
  policy_id: string | null
  status: 'analyzed' | 'reviewed' | 'escalated'
  decision: Decision
  final_decision: Decision | null
  confidence: number
  claimed_total: number
  deductions: number
  payable: number | null
  engine_version: string
  version: number
  created_by: string
  created_by_name: string | null
  created_at: number
  updated_at: number
  payload_hash: string
  analysis_hash: string
}

export type FindingStatus =
  | 'SUPPORTS_ADMISSIBLE'
  | 'SUPPORTS_LIMIT'
  | 'SUPPORTS_EXCLUSION'
  | 'INSUFFICIENT_EVIDENCE'
  | 'NOT_APPLICABLE'

export interface Finding {
  dimension: string
  status: FindingStatus
  statement: string
  confidence: number
  applicable: boolean
  chunk_ids: string[]
  assumptions: string[]
  source: 'rules' | 'llm'
}

export interface Citation {
  claim: string
  source: string
  page: number
  section: string
  chunk_id: string
  rerank_score: number | null
  excerpt: string | null
}

export interface CheckResult {
  name: string
  passed: boolean
  detail: string
}

export type Verdict = 'SUPPORTED' | 'UNSUPPORTED' | 'CONTRADICTED'

export interface ClaimVerification {
  claim_id: string
  kind: 'policy_claim' | 'limit_claim' | 'decision_claim'
  dimension: string
  claim: string
  chunk_id: string | null
  page: number | null
  section: string | null
  verdict: Verdict
  reason: string
  checks: CheckResult[]
  excerpt: string | null
}

export interface TraceEvent {
  agent: string
  action: string
  detail: string
  elapsed_ms: number
  retrieval_count: number | null
  attempt: number
  reads: string[]
  writes: string[]
}

export interface Handoff {
  from_agent: string
  to_agent: string
  payload: string
  kind: 'forward' | 'retry'
}

export interface Analysis {
  case_id: string
  decision: Decision
  confidence: number
  key_findings: string[]
  findings: Finding[]
  assumptions: { dimension: string; assumption: string }[]
  unmodelled_policy_risks: { statement: string; chunk_ids: string[] }[]
  amounts: {
    claimed_total_inr: number
    total_deductions_inr: number
    estimated_payable_inr: number | null
    expense_breakdown_inr: Record<string, number>
  }
  applicable_limits: { description: string; deduction_inr: number | null; dimension: string }[]
  missing_evidence: { field: string; reason: string }[]
  citations: Citation[]
  validation: {
    status: 'PASS' | 'FAIL'
    unsupported_claims: string[]
    notes: string
    verifications: ClaimVerification[]
    counts: Record<Verdict, number>
    attempts: number
  }
  rationale: string
  rationale_source: string
  llm_interpretation: { enabled: boolean; accepted: unknown[]; rejected: unknown[]; error: string | null }
  trace: TraceEvent[]
  handoffs: Handoff[]
  unrecognized_fields: string[]
  engine_version: string
}

export interface LedgerEntry {
  seq: number
  claim_id: string
  kind: 'claim_analyzed' | 'claim_reanalyzed' | 'claim_reviewed'
  payload_hash: string
  prev_hash: string
  entry_hash: string
  actor_id: string
  cid: string | null
  chain_status: 'local' | 'pending' | 'retry' | 'pinned' | 'anchored' | 'failed'
  tx_hash: string | null
  block_number: number | null
  attempts: number
  last_error: string | null
  created_at: number
  anchored_at: number | null
  tx_url?: string | null
  ipfs_url?: string | null
  record_id?: string
}

export interface Review {
  id: string
  reviewer_id: string
  reviewer_name: string | null
  action: 'agree' | 'override' | 'escalate'
  final_decision: Decision
  notes: string
  created_at: number
}

export interface ClaimDetail {
  claim: ClaimSummary
  case: Record<string, unknown>
  analysis: Analysis
  reviews: Review[]
  ledger: LedgerEntry[]
}

export interface Proof {
  claim_id: string
  ok: boolean
  checks: CheckResult[]
  entries: LedgerEntry[]
  record_id: string
  chain: { contract: string; chain_id: number } | null
  verified_at: number
}

export interface Paged<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}

export interface Sample {
  case_id: string
  task: string | null
  diagnosis: string | null
  expected_decision: Decision | null
  case: Record<string, unknown>
}

export interface Stats {
  scope: 'all' | 'mine'
  stats: {
    total_claims: number
    by_decision: [Decision, number][]
    by_status: [string, number][]
    claimed_total: number
    payable_total: number
    ledger_entries: number
    ledger_by_status: [string, number][]
  }
}

export interface PolicyChunk {
  chunk_id: string
  section: string
  label: string
  page_start: number
  page_end: number
  text: string
}

export interface EvidenceItem extends PolicyChunk {
  dense_rank: number | null
  dense_score: number | null
  sparse_rank: number | null
  sparse_score: number | null
  fused_score: number | null
  rerank_score: number | null
}

export interface FieldError {
  field: string
  message: string
}
