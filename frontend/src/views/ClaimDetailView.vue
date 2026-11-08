<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api, ApiError, downloadJson } from '@/api/client'
import type { ClaimDetail } from '@/api/types'
import DecisionStamp from '@/components/DecisionStamp.vue'
import DecisionTag from '@/components/DecisionTag.vue'
import EmptyState from '@/components/EmptyState.vue'
import TabStrip from '@/components/TabStrip.vue'
import CitationsList from '@/components/claim/CitationsList.vue'
import FindingsList from '@/components/claim/FindingsList.vue'
import LedgerProof from '@/components/claim/LedgerProof.vue'
import ReviewPanel from '@/components/claim/ReviewPanel.vue'
import VerificationTable from '@/components/claim/VerificationTable.vue'
import WorkflowSlip from '@/components/claim/WorkflowSlip.vue'
import { useToasts } from '@/stores/toast'
import { DECISION_LABEL, humanize, money, pct, when } from '@/utils/format'

const props = defineProps<{ id: string }>()
const route = useRoute()
const toasts = useToasts()
const data = ref<ClaimDetail | null>(null)
const error = ref('')
const notFound = ref(false)
const tab = ref('findings')
const press = ref(route.query.fresh === '1')
const reanalysing = ref(false)

async function load() {
  error.value = ''
  try {
    data.value = await api.get<ClaimDetail>(`/api/claims/${props.id}`)
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) notFound.value = true
    else error.value = (e as Error).message
  }
}
watch(() => props.id, load, { immediate: true })

const a = computed(() => data.value?.analysis)
interface CaseFile {
  policy_id?: string
  policy_start_date?: string
  claim_date?: string
  sum_insured_inr?: number
  continuous_coverage_months?: number
  prior_insurer_continuous_years?: number
  hospital?: { name?: string; network_provider?: boolean }
  treatment?: { type?: string; admission_hours?: number; diagnosis?: string; procedure?: string }
}
const c = computed(() => data.value?.case as CaseFile | undefined)
const tabs = computed(() => [
  { id: 'findings', label: 'Findings', count: a.value?.findings.filter((f) => f.applicable).length },
  { id: 'evidence', label: 'Policy evidence', count: a.value?.citations.length },
  { id: 'verification', label: 'Verification', count: a.value?.validation.verifications.length },
  { id: 'workflow', label: 'Agent workflow' },
  { id: 'ledger', label: 'Ledger', count: data.value?.ledger.length },
  { id: 'review', label: 'Review', count: data.value?.reviews.length },
])
const caption = computed(() => {
  if (!a.value) return ''
  return a.value.amounts.estimated_payable_inr === null ? 'No amount until reviewed' : `Pay ${money(a.value.amounts.estimated_payable_inr)}`
})

async function reanalyse() {
  reanalysing.value = true
  try {
    await api.post(`/api/claims/${props.id}/reanalyze`, {})
    toasts.push('Re-analysed and written to the ledger.', 'ok')
    press.value = true
    await load()
  } catch (e) {
    toasts.push((e as Error).message, 'bad')
  } finally {
    reanalysing.value = false
  }
}
</script>

<template>
  <div class="page">
    <EmptyState v-if="notFound" title="Claim not found" body="It may not exist, or it belongs to another analyst.">
      <RouterLink to="/claims" class="btn">Back to claims</RouterLink>
    </EmptyState>
    <p v-else-if="error" class="error-text">{{ error }}</p>

    <template v-else-if="data && a && c">
      <nav class="crumbs"><RouterLink to="/claims">Claims</RouterLink> / {{ data.claim.case_id }}</nav>

      <header class="head">
        <div class="head-main">
          <h1>{{ data.claim.case_id }}</h1>
          <p class="muted">{{ c.treatment?.diagnosis || 'No diagnosis given' }}<template v-if="c.treatment?.procedure"> · {{ c.treatment.procedure }}</template></p>
          <dl class="meta">
            <div><dt>Confidence</dt><dd>{{ pct(a.confidence) }}</dd></div>
            <div><dt>Claimed</dt><dd>{{ money(a.amounts.claimed_total_inr) }}</dd></div>
            <div><dt>Deductions</dt><dd>{{ money(a.amounts.total_deductions_inr) }}</dd></div>
            <div><dt>Payable</dt><dd>{{ money(a.amounts.estimated_payable_inr) }}</dd></div>
          </dl>
          <p v-if="data.claim.final_decision" class="final">
            Reviewer's decision: <DecisionTag :decision="data.claim.final_decision" />
            <span class="faint">({{ data.claim.status === 'escalated' ? 'escalated' : 'signed off' }})</span>
          </p>
        </div>
        <div class="head-stamp">
          <DecisionStamp :decision="a.decision" :press="press" :caption="caption" />
          <div class="head-actions">
            <button class="btn" type="button" :disabled="reanalysing" @click="reanalyse">{{ reanalysing ? 'Re-analysing…' : 'Re-analyse' }}</button>
            <button class="btn" type="button" @click="downloadJson(`/api/claims/${id}/export`, `claim-${data.claim.case_id}.json`)">Export audit file</button>
          </div>
        </div>
      </header>

      <div class="body">
        <aside class="file" aria-label="Case file">
          <h2>Case file</h2>
          <dl class="facts">
            <dt>Policy</dt><dd>{{ c.policy_id ?? '—' }}</dd>
            <dt>Policy start</dt><dd>{{ c.policy_start_date }}</dd>
            <dt>Claim date</dt><dd>{{ c.claim_date }}</dd>
            <dt>Sum insured</dt><dd>{{ money(c.sum_insured_inr) }}</dd>
            <dt>Continuous cover</dt><dd>{{ c.continuous_coverage_months ?? 0 }} months</dd>
            <dt>Prior insurer</dt><dd>{{ c.prior_insurer_continuous_years ?? 0 }} years</dd>
            <dt>Hospital</dt><dd>{{ c.hospital?.name || '—' }}<span v-if="c.hospital?.network_provider" class="faint"> (network)</span></dd>
            <dt>Treatment</dt><dd>{{ humanize(String(c.treatment?.type ?? 'inpatient')) }}, {{ c.treatment?.admission_hours ?? 24 }} h</dd>
          </dl>
          <h3>Expenses</h3>
          <table class="table compact">
            <tbody>
              <tr v-for="(v, k) in a.amounts.expense_breakdown_inr" :key="k"><td>{{ humanize(String(k)) }}</td><td class="num">{{ money(v) }}</td></tr>
            </tbody>
          </table>
          <p v-if="a.unrecognized_fields.length" class="faint small">Ignored fields: {{ a.unrecognized_fields.join(', ') }}</p>
          <p class="faint small">Engine {{ a.engine_version }} · version {{ data.claim.version }} · {{ when(data.claim.updated_at) }}</p>
        </aside>

        <section class="work">
          <TabStrip v-model="tab" :tabs="tabs" />
          <div :id="`panel-${tab}`" role="tabpanel" class="panel-body">
            <FindingsList v-if="tab === 'findings'" :analysis="a" />
            <CitationsList v-else-if="tab === 'evidence'" :citations="a.citations" />
            <VerificationTable v-else-if="tab === 'verification'" :validation="a.validation" />
            <WorkflowSlip v-else-if="tab === 'workflow'" :analysis="a" />
            <LedgerProof v-else-if="tab === 'ledger'" :claim-id="id" :entries="data.ledger" />
            <ReviewPanel v-else :claim="data.claim" :reviews="data.reviews" @done="load" />
          </div>
        </section>
      </div>
      <p class="sr-only" aria-live="polite">Decision {{ DECISION_LABEL[a.decision] }}</p>
    </template>
  </div>
</template>

<style scoped>
.crumbs {
  font-size: var(--t-sm);
  color: var(--ink-faint);
  margin-bottom: 1rem;
}

.head {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 2rem;
  padding-bottom: 1.75rem;
  border-bottom: 1px solid var(--rule-strong);
}

.head-main {
  display: grid;
  gap: 0.5rem;
  align-content: start;
  min-width: 0;
}

.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem 2.25rem;
  margin: 0.75rem 0 0;
}

.meta div {
  display: grid;
}

.meta dt {
  font-size: var(--t-xs);
  color: var(--ink-faint);
}

.meta dd {
  margin: 0;
  font-weight: 640;
  font-size: var(--t-lg);
}

.final {
  display: flex;
  gap: 0.6rem;
  align-items: center;
  font-size: var(--t-sm);
}

.head-stamp {
  display: grid;
  justify-items: end;
  gap: 1.25rem;
  padding: 0.5rem 1rem 0 0;
}

.head-actions {
  display: flex;
  gap: 0.5rem;
}

.body {
  display: grid;
  grid-template-columns: 18rem minmax(0, 1fr);
  gap: 2.5rem;
  margin-top: 1.75rem;
  align-items: start;
}

.file {
  position: sticky;
  top: 1.5rem;
  display: grid;
  gap: 0.75rem;
  padding: 1.25rem;
  background: var(--surface);
  border: 1px solid var(--rule);
  border-radius: var(--r-md);
}

.facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.35rem 0.85rem;
  margin: 0;
  font-size: var(--t-sm);
}

.facts dt {
  color: var(--ink-faint);
}

.facts dd {
  margin: 0;
  text-align: right;
  overflow-wrap: anywhere;
}

.compact td {
  padding: 0.35rem 0;
}

.small {
  font-size: var(--t-xs);
}

.panel-body {
  padding-top: 1.5rem;
}

@media (max-width: 1000px) {
  .body {
    grid-template-columns: minmax(0, 1fr);
  }

  .file {
    position: static;
  }

  .head-stamp {
    justify-items: start;
  }
}
</style>
