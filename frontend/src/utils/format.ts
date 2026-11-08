import type { Decision, FindingStatus, Role } from '@/api/types'

const inr = new Intl.NumberFormat('en-IN', { maximumFractionDigits: 0 })

/** "₹1,90,500" (Indian digit grouping); em dash for null. */
export function money(v: number | null | undefined): string {
  if (v === null || v === undefined || Number.isNaN(v)) return '—'
  return `₹${inr.format(Math.round(v) || 0)}`
}

export function pct(v: number): string {
  return `${Math.round(v * 100)}%`
}

export function when(ms: number | null | undefined): string {
  if (!ms) return '—'
  return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
}

export function shortHash(h: string | null | undefined, n = 10): string {
  if (!h) return '—'
  return h.length > n * 2 ? `${h.slice(0, n)}…${h.slice(-4)}` : h
}

export const DECISION_LABEL: Record<Decision, string> = {
  ADMISSIBLE: 'Admissible',
  ADMISSIBLE_WITH_LIMITS: 'Admissible with limits',
  PARTIALLY_ADMISSIBLE: 'Partially admissible',
  NOT_ADMISSIBLE: 'Not admissible',
  NEEDS_REVIEW: 'Needs review',
}

/** Short wording for the stamp. */
export const DECISION_STAMP: Record<Decision, string> = {
  ADMISSIBLE: 'Admissible',
  ADMISSIBLE_WITH_LIMITS: 'With limits',
  PARTIALLY_ADMISSIBLE: 'Partial',
  NOT_ADMISSIBLE: 'Not admissible',
  NEEDS_REVIEW: 'Hold for review',
}

export const DECISION_TONE: Record<Decision, string> = {
  ADMISSIBLE: 'var(--d-admissible)',
  ADMISSIBLE_WITH_LIMITS: 'var(--d-limits)',
  PARTIALLY_ADMISSIBLE: 'var(--d-partial)',
  NOT_ADMISSIBLE: 'var(--d-not)',
  NEEDS_REVIEW: 'var(--d-review)',
}

export const DECISIONS = Object.keys(DECISION_LABEL) as Decision[]

export const STATUS_LABEL: Record<FindingStatus, string> = {
  SUPPORTS_ADMISSIBLE: 'Satisfied',
  SUPPORTS_LIMIT: 'Limit applies',
  SUPPORTS_EXCLUSION: 'Excludes',
  INSUFFICIENT_EVIDENCE: 'Unresolved',
  NOT_APPLICABLE: 'Not triggered',
}

export const ROLE_LABEL: Record<Role, string> = { analyst: 'Analyst', reviewer: 'Reviewer', admin: 'Administrator' }

/** "named_disease_first_year_waiting_period" -> "Named disease first year waiting period" */
export function humanize(key: string): string {
  const s = key.replace(/_/g, ' ').trim()
  return s.charAt(0).toUpperCase() + s.slice(1)
}

export const AGENT_LABEL: Record<string, string> = {
  CaseAnalysisAgent: 'Case analysis',
  PolicyEvidenceAgent: 'Policy evidence',
  CoverageExclusionAgent: 'Coverage & exclusion',
  DecisionAgent: 'Decision',
  ValidationAgent: 'Validation',
  Response: 'Response',
}
