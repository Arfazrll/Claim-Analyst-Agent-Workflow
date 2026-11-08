<script setup lang="ts">
import { ref } from 'vue'
import { api } from '@/api/client'
import type { Decision } from '@/api/types'
import DecisionTag from '@/components/DecisionTag.vue'
import { DECISIONS, DECISION_LABEL, money } from '@/utils/format'

interface CaseResult {
  case_id: string
  expected: Decision | null
  actual: Decision
  correct: boolean | null
  unsafe_decision: boolean
  expected_deduction_inr: number | null
  actual_deduction_inr: number
  amount_match: boolean | null
  claims_supported: number
  claims_total: number
  attempts: number
  latency_ms: number
}
interface Report {
  engine_version: string
  labelled_cases: number
  correct: number
  accuracy: number
  unsafe_decisions: number
  required_abstentions: number
  abstentions_caught: number
  amount_cases: number
  amount_exact: number
  claims_verified: number
  claims_total: number
  faults_injected: number
  faults_detected: number
  deterministic: boolean
  latency_p50_ms: number
  latency_p95_ms: number
  confusion: Record<string, Record<string, number>>
  cases: CaseResult[]
}

const report = ref<Report | null>(null)
const busy = ref(false)
const error = ref('')

async function run() {
  busy.value = true
  error.value = ''
  try {
    report.value = await api.post<Report>('/api/admin/eval')
  } catch (e) {
    error.value = (e as Error).message
  } finally {
    busy.value = false
  }
}
run()
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Evaluation</h1>
        <p>The bundled labelled cases, run through the full pipeline offline. The same suite gates every CI build.</p>
      </div>
      <button class="btn" type="button" :disabled="busy" @click="run">{{ busy ? 'Running…' : 'Run again' }}</button>
    </header>
    <p v-if="error" class="error-text">{{ error }}</p>

    <template v-if="report">
      <dl class="scores">
        <div><dt>Decisions correct</dt><dd>{{ report.correct }}/{{ report.labelled_cases }}</dd></div>
        <div><dt>Unsafe decisions</dt><dd :class="{ 'error-text': report.unsafe_decisions }">{{ report.unsafe_decisions }}</dd></div>
        <div><dt>Required reviews caught</dt><dd>{{ report.abstentions_caught }}/{{ report.required_abstentions }}</dd></div>
        <div><dt>Exact payable amounts</dt><dd>{{ report.amount_exact }}/{{ report.amount_cases }}</dd></div>
        <div><dt>Claims verified</dt><dd>{{ report.claims_verified }}/{{ report.claims_total }}</dd></div>
        <div><dt>Injected faults caught</dt><dd>{{ report.faults_detected }}/{{ report.faults_injected }}</dd></div>
        <div><dt>Deterministic</dt><dd>{{ report.deterministic ? 'Yes' : 'No' }}</dd></div>
        <div><dt>Latency p50 / p95</dt><dd>{{ report.latency_p50_ms.toFixed(1) }} / {{ report.latency_p95_ms.toFixed(1) }} ms</dd></div>
      </dl>

      <h2>Confusion matrix</h2>
      <p class="muted small">Rows are the hand-derived label, columns the engine's decision.</p>
      <div class="table-wrap">
        <table class="table matrix">
          <thead><tr><th>Expected</th><th v-for="d in DECISIONS" :key="d" class="num">{{ DECISION_LABEL[d] }}</th></tr></thead>
          <tbody>
            <tr v-for="row in DECISIONS" :key="row">
              <th scope="row">{{ DECISION_LABEL[row] }}</th>
              <td v-for="col in DECISIONS" :key="col" class="num" :class="{ diag: row === col, off: row !== col && report.confusion[row]?.[col] }">{{ report.confusion[row]?.[col] ?? '' }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <h2>Cases</h2>
      <div class="table-wrap">
        <table class="table">
          <thead><tr><th>Case</th><th>Expected</th><th>Engine</th><th class="num">Expected deduction</th><th class="num">Deduction</th><th class="num">Claims verified</th><th class="num">ms</th></tr></thead>
          <tbody>
            <tr v-for="c in report.cases" :key="c.case_id" :class="{ miss: c.correct === false }">
              <td><strong>{{ c.case_id }}</strong></td>
              <td><DecisionTag v-if="c.expected" :decision="c.expected" /><span v-else class="faint">unlabelled</span></td>
              <td><DecisionTag :decision="c.actual" /></td>
              <td class="num">{{ c.expected_deduction_inr === null ? '—' : money(c.expected_deduction_inr) }}</td>
              <td class="num">{{ money(c.actual_deduction_inr) }}</td>
              <td class="num">{{ c.claims_supported }}/{{ c.claims_total }}</td>
              <td class="num faint">{{ c.latency_ms.toFixed(1) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="faint small">Engine {{ report.engine_version }}. Labels are a reading of the policy wording, not an insurer's adjudication.</p>
    </template>
  </div>
</template>

<style scoped>
.scores {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
  gap: 1.25rem 2rem;
  margin: 0 0 2.5rem;
  padding: 1.5rem 0;
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule);
}

.scores dt {
  font-size: var(--t-xs);
  color: var(--ink-faint);
}

.scores dd {
  margin: 0;
  font-size: var(--t-xl);
  font-weight: 650;
  font-stretch: 112%;
}

h2 {
  margin-top: 1rem;
}

.small {
  font-size: var(--t-sm);
  margin: 0.3rem 0 0.75rem;
}

.matrix th[scope='row'] {
  font-size: var(--t-sm);
  color: var(--ink);
  border-bottom: 1px solid var(--rule);
}

.diag {
  font-weight: 700;
  color: var(--ok);
}

.off {
  color: var(--bad);
  font-weight: 700;
}

.miss {
  background: color-mix(in srgb, var(--bad) 8%, transparent);
}

.table-wrap {
  margin-bottom: 2rem;
}
</style>
