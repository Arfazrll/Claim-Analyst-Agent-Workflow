<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { api, ApiError } from '@/api/client'
import type { ClaimSummary, FieldError, Sample } from '@/api/types'
import DecisionTag from '@/components/DecisionTag.vue'
import TabStrip from '@/components/TabStrip.vue'
import { useToasts } from '@/stores/toast'

const router = useRouter()
const toasts = useToasts()
const mode = ref<'form' | 'json'>('form')
const samples = ref<Sample[]>([])
const busy = ref(false)
const error = ref('')
const fieldErrors = ref<FieldError[]>([])
const interpret = ref(false)
const jsonText = ref('')
const JSON_HINT = '{ "case_id": "CLM-1", "policy_start_date": "2025-01-01", "claim_date": "2026-03-14", "treatment": { "type": "inpatient" } }'

const DOCS = ['claim_form', 'discharge_summary', 'itemized_bill', 'doctor_prescription', 'prior_medical_records', 'pre_post_expense_records']
const EXPENSES = [
  ['room', 'Room, boarding, nursing'],
  ['doctor_fees', 'Doctor and surgeon fees'],
  ['medicines_diagnostics', 'Medicines and diagnostics'],
  ['pre_hospitalization', 'Pre-hospitalisation'],
  ['post_hospitalization', 'Post-hospitalisation'],
  ['ambulance', 'Ambulance'],
] as const

const form = reactive({
  case_id: '',
  policy_id: 'USGIC-CSC-2017-2018',
  policy_start_date: '',
  claim_date: '',
  sum_insured_inr: 500000,
  continuous_coverage_months: 0,
  prior_insurer_continuous_years: 0,
  age: 35,
  hospital_name: '',
  network_provider: true,
  type: 'inpatient',
  admission_hours: 72,
  diagnosis: '',
  procedure: '',
  pre_existing: false,
  experimental: false,
  hospital_room_unavailable: null as boolean | null,
  patient_cannot_be_moved: null as boolean | null,
  expenses: Object.fromEntries(EXPENSES.map(([k]) => [k, 0])) as Record<string, number>,
  documents: ['claim_form', 'discharge_summary', 'itemized_bill'] as string[],
})

const total = computed(() => Object.values(form.expenses).reduce((a, b) => a + (Number(b) || 0), 0))

function buildCase(): Record<string, unknown> {
  const c: Record<string, unknown> = {
    case_id: form.case_id.trim(),
    policy_id: form.policy_id.trim() || undefined,
    policy_start_date: form.policy_start_date,
    claim_date: form.claim_date,
    sum_insured_inr: Number(form.sum_insured_inr),
    continuous_coverage_months: Number(form.continuous_coverage_months),
    prior_insurer_continuous_years: Number(form.prior_insurer_continuous_years),
    patient: { age: Number(form.age) },
    hospital: { name: form.hospital_name.trim(), network_provider: form.network_provider },
    treatment: {
      type: form.type,
      admission_hours: Number(form.admission_hours),
      diagnosis: form.diagnosis.trim(),
      procedure: form.procedure.trim(),
      pre_existing: form.pre_existing,
      experimental: form.experimental,
      ...(form.type === 'domiciliary'
        ? { hospital_room_unavailable: form.hospital_room_unavailable, patient_cannot_be_moved: form.patient_cannot_be_moved }
        : {}),
    },
    expenses_inr: Object.fromEntries(Object.entries(form.expenses).map(([k, v]) => [k, Number(v) || 0])),
    documents: form.documents,
  }
  return c
}

function loadSample(s: Sample) {
  jsonText.value = JSON.stringify(s.case, null, 2)
  mode.value = 'json'
  error.value = ''
  fieldErrors.value = []
}

onMounted(async () => {
  try {
    samples.value = (await api.get<{ items: Sample[] }>('/api/samples')).items
  } catch {
    samples.value = []
  }
})

function errorFor(field: string) {
  return fieldErrors.value.find((f) => f.field === field || f.field.startsWith(`${field}.`))?.message
}

async function submit() {
  error.value = ''
  fieldErrors.value = []
  let payload: unknown
  if (mode.value === 'json') {
    try {
      payload = JSON.parse(jsonText.value)
    } catch {
      error.value = 'The JSON is not valid. Check for a missing comma or quote.'
      return
    }
  } else {
    payload = buildCase()
  }
  busy.value = true
  try {
    const key = crypto.randomUUID()
    const res = await api.post<{ claim: ClaimSummary }>('/api/claims', { case: payload, llm_interpretation: interpret.value }, { 'Idempotency-Key': key })
    toasts.push(`Claim ${res.claim.case_id} analysed.`, 'ok')
    router.push({ path: `/claims/${res.claim.id}`, query: { fresh: '1' } })
  } catch (e) {
    if (e instanceof ApiError) {
      error.value = e.status === 429 ? `Too many analyses. Try again in ${e.retryAfter ?? 60} s.` : e.message
      fieldErrors.value = e.fieldErrors
    } else {
      error.value = 'The claim could not be analysed.'
    }
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Analyse a claim</h1>
        <p>Enter the case, or start from a sample. The engine cites the policy clause behind every finding.</p>
      </div>
    </header>

    <div class="layout">
      <form class="main" novalidate @submit.prevent="submit">
        <TabStrip v-model="mode" :tabs="[{ id: 'form', label: 'Form' }, { id: 'json', label: 'JSON' }]" />

        <div v-if="mode === 'form'" id="panel-form" role="tabpanel" class="sections">
          <fieldset>
            <legend>Policy</legend>
            <div class="grid">
              <label class="field"><span>Case ID</span><input v-model="form.case_id" class="input" required maxlength="128" placeholder="CLM-2026-0142" /><small v-if="errorFor('case_id')" class="error-text">{{ errorFor('case_id') }}</small></label>
              <label class="field"><span>Policy ID</span><input v-model="form.policy_id" class="input" /></label>
              <label class="field"><span>Policy start date</span><input v-model="form.policy_start_date" class="input" type="date" required /><small v-if="errorFor('policy_start_date')" class="error-text">{{ errorFor('policy_start_date') }}</small></label>
              <label class="field"><span>Claim date</span><input v-model="form.claim_date" class="input" type="date" required /><small v-if="errorFor('claim_date')" class="error-text">{{ errorFor('claim_date') }}</small></label>
              <label class="field"><span>Sum insured (₹)</span><input v-model.number="form.sum_insured_inr" class="input" type="number" min="0" step="1000" /></label>
              <label class="field"><span>Continuous coverage (months)</span><input v-model.number="form.continuous_coverage_months" class="input" type="number" min="0" /></label>
              <label class="field"><span>Prior insurer (completed years)</span><input v-model.number="form.prior_insurer_continuous_years" class="input" type="number" min="0" /></label>
              <label class="field"><span>Patient age</span><input v-model.number="form.age" class="input" type="number" min="0" max="120" /></label>
            </div>
          </fieldset>

          <fieldset>
            <legend>Treatment</legend>
            <div class="grid">
              <label class="field"><span>Hospital</span><input v-model="form.hospital_name" class="input" maxlength="120" /></label>
              <label class="field">
                <span>Treatment type</span>
                <select v-model="form.type" class="select">
                  <option value="inpatient">In-patient</option>
                  <option value="day_care">Day care</option>
                  <option value="domiciliary">Domiciliary (at home)</option>
                </select>
              </label>
              <label class="field"><span>Admission (hours)</span><input v-model.number="form.admission_hours" class="input" type="number" min="0" /></label>
              <label class="field wide"><span>Diagnosis</span><input v-model="form.diagnosis" class="input" maxlength="200" placeholder="Acute appendicitis" /></label>
              <label class="field wide"><span>Procedure</span><input v-model="form.procedure" class="input" maxlength="200" placeholder="Laparoscopic appendectomy" /></label>
            </div>
            <div class="checks">
              <label class="check"><input v-model="form.network_provider" type="checkbox" /> Network hospital</label>
              <label class="check"><input v-model="form.pre_existing" type="checkbox" /> Pre-existing condition</label>
              <label class="check"><input v-model="form.experimental" type="checkbox" /> Experimental treatment</label>
            </div>
            <div v-if="form.type === 'domiciliary'" class="grid">
              <label class="field">
                <span>Hospital room unavailable?</span>
                <select v-model="form.hospital_room_unavailable" class="select"><option :value="null">Not stated</option><option :value="true">Yes</option><option :value="false">No</option></select>
              </label>
              <label class="field">
                <span>Patient could not be moved?</span>
                <select v-model="form.patient_cannot_be_moved" class="select"><option :value="null">Not stated</option><option :value="true">Yes</option><option :value="false">No</option></select>
              </label>
            </div>
          </fieldset>

          <fieldset>
            <legend>Expenses</legend>
            <div class="grid">
              <label v-for="[k, label] in EXPENSES" :key="k" class="field"><span>{{ label }} (₹)</span><input v-model.number="form.expenses[k]" class="input" type="number" min="0" step="100" /></label>
            </div>
            <p class="muted total">Total claimed: <strong>₹{{ total.toLocaleString('en-IN') }}</strong></p>
          </fieldset>

          <fieldset>
            <legend>Documents supplied</legend>
            <div class="checks">
              <label v-for="d in DOCS" :key="d" class="check"><input v-model="form.documents" type="checkbox" :value="d" /> {{ d.replace(/_/g, ' ') }}</label>
            </div>
          </fieldset>
        </div>

        <div v-else id="panel-json" role="tabpanel" class="sections">
          <label class="field">
            <span>Claim case JSON</span>
            <textarea v-model="jsonText" class="textarea code" spellcheck="false" :placeholder="JSON_HINT"></textarea>
            <small>Unknown fields are kept and reported, never used in the decision.</small>
          </label>
        </div>

        <ul v-if="fieldErrors.length" class="error-text errs" role="alert">
          <li v-for="(f, i) in fieldErrors" :key="i"><code>{{ f.field }}</code>: {{ f.message }}</li>
        </ul>
        <p v-if="error" class="error-text" role="alert">{{ error }}</p>

        <div class="submit">
          <label class="check"><input v-model="interpret" type="checkbox" /> Also ask the language model to look for exclusions the rules miss (can only hold a claim for review)</label>
          <button class="btn btn-primary" type="submit" :disabled="busy">{{ busy ? 'Analysing…' : 'Analyse claim' }}</button>
        </div>
      </form>

      <aside class="samples" aria-label="Sample cases">
        <h2>Sample cases</h2>
        <p class="muted">Labelled test cases from the evaluation set. Open one to see how the engine handles it.</p>
        <ul>
          <li v-for="s in samples" :key="s.case_id">
            <button type="button" @click="loadSample(s)">
              <span class="s-id">{{ s.case_id }}</span>
              <span class="s-dx">{{ s.diagnosis }}</span>
              <DecisionTag v-if="s.expected_decision" :decision="s.expected_decision" />
            </button>
          </li>
        </ul>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 20rem;
  gap: 2.5rem;
  align-items: start;
}

.main {
  display: grid;
  gap: 1.5rem;
}

.sections {
  display: grid;
  gap: 1.75rem;
  padding-top: 0.5rem;
}

fieldset {
  border: 0;
  padding: 0;
  margin: 0;
  display: grid;
  gap: 1rem;
}

legend {
  font-stretch: 112%;
  font-weight: 640;
  font-size: var(--t-md);
  margin-bottom: 0.75rem;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(13rem, 1fr));
  gap: 1rem;
}

.wide {
  grid-column: span 2;
}

.checks {
  display: flex;
  flex-wrap: wrap;
  gap: 0.6rem 1.5rem;
}

.total {
  font-size: var(--t-sm);
}

.errs {
  margin: 0;
  padding-left: 1.1rem;
}

.submit {
  display: grid;
  gap: 1rem;
  justify-items: start;
  padding-top: 1rem;
  border-top: 1px solid var(--rule);
}

.samples {
  position: sticky;
  top: 1.5rem;
  max-height: calc(100vh - 3rem);
  overflow: auto;
}

.samples > p {
  font-size: var(--t-sm);
  margin: 0.3rem 0 0.9rem;
}

.samples ul {
  list-style: none;
  margin: 0;
  padding: 0;
}

.samples button {
  width: 100%;
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.1rem 0.75rem;
  text-align: left;
  font: inherit;
  background: none;
  border: 0;
  border-bottom: 1px solid var(--rule);
  padding: 0.6rem 0.25rem;
  color: var(--ink);
  cursor: pointer;
}

.samples button:hover {
  background: var(--surface-sunk);
}

.s-id {
  font-weight: 640;
  font-size: var(--t-sm);
}

.s-dx {
  font-size: var(--t-sm);
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.samples :deep(.tag) {
  grid-column: 2;
}

@media (max-width: 1000px) {
  .layout {
    grid-template-columns: 1fr;
  }

  .samples {
    position: static;
    max-height: none;
    order: -1;
  }

  .wide {
    grid-column: auto;
  }
}
</style>
