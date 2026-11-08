<script setup lang="ts">
import { ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '@/api/client'
import type { ClaimSummary, Paged } from '@/api/types'
import DecisionTag from '@/components/DecisionTag.vue'
import EmptyState from '@/components/EmptyState.vue'
import PagerNav from '@/components/PagerNav.vue'
import { DECISIONS, DECISION_LABEL, money, when } from '@/utils/format'

const route = useRoute()
const router = useRouter()
const data = ref<Paged<ClaimSummary> | null>(null)
const loading = ref(false)
const error = ref('')

const q = ref(String(route.query.q ?? ''))
const decision = ref(String(route.query.decision ?? ''))
const status = ref(String(route.query.status ?? ''))
const page = ref(Number(route.query.page ?? 1) || 1)

let timer: number | undefined
async function load() {
  loading.value = true
  error.value = ''
  const params = new URLSearchParams({ page: String(page.value), page_size: '20' })
  if (q.value.trim()) params.set('q', q.value.trim())
  if (decision.value) params.set('decision', decision.value)
  if (status.value) params.set('status', status.value)
  router.replace({ query: Object.fromEntries(params) })
  try {
    data.value = await api.get<Paged<ClaimSummary>>(`/api/claims?${params}`)
  } catch (e) {
    error.value = (e as Error).message
  } finally {
    loading.value = false
  }
}

watch([decision, status], () => {
  page.value = 1
  load()
})
watch(q, () => {
  window.clearTimeout(timer)
  timer = window.setTimeout(() => {
    page.value = 1
    load()
  }, 300)
})
load()

const STATUS_TEXT: Record<string, string> = { analyzed: 'Awaiting review', reviewed: 'Signed off', escalated: 'Escalated' }
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Claims</h1>
        <p>Every analysis is stored encrypted and sealed into the ledger.</p>
      </div>
      <RouterLink to="/claims/new" class="btn btn-primary">Analyse a claim</RouterLink>
    </header>

    <div class="filters">
      <label class="field grow"><span>Case ID</span><input v-model="q" class="input" type="search" placeholder="Search by case ID" /></label>
      <label class="field">
        <span>Decision</span>
        <select v-model="decision" class="select">
          <option value="">Any decision</option>
          <option v-for="d in DECISIONS" :key="d" :value="d">{{ DECISION_LABEL[d] }}</option>
        </select>
      </label>
      <label class="field">
        <span>Status</span>
        <select v-model="status" class="select">
          <option value="">Any status</option>
          <option value="analyzed">Awaiting review</option>
          <option value="reviewed">Signed off</option>
          <option value="escalated">Escalated</option>
        </select>
      </label>
    </div>

    <p v-if="error" class="error-text">{{ error }}</p>
    <EmptyState v-else-if="data && !data.items.length" title="No claims match" body="Change the filters, or analyse a new claim.">
      <RouterLink to="/claims/new" class="btn">Analyse a claim</RouterLink>
    </EmptyState>
    <div v-else class="table-wrap" :aria-busy="loading">
      <table class="table">
        <thead>
          <tr><th>Case</th><th>Engine decision</th><th>Status</th><th class="num">Claimed</th><th class="num">Payable</th><th>Submitted</th></tr>
        </thead>
        <tbody>
          <tr v-for="c in data?.items ?? []" :key="c.id">
            <td><RouterLink :to="`/claims/${c.id}`" class="case">{{ c.case_id }}</RouterLink><div class="faint small">{{ c.created_by_name }}</div></td>
            <td><DecisionTag :decision="c.decision" /><div v-if="c.final_decision && c.final_decision !== c.decision" class="small faint">Reviewer: {{ DECISION_LABEL[c.final_decision] }}</div></td>
            <td>{{ STATUS_TEXT[c.status] ?? c.status }}</td>
            <td class="num">{{ money(c.claimed_total) }}</td>
            <td class="num">{{ money(c.payable) }}</td>
            <td class="faint">{{ when(c.created_at) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <PagerNav v-if="data" :page="data.page" :page-size="data.page_size" :total="data.total" @go="(p) => { page = p; load() }" />
  </div>
</template>

<style scoped>
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.filters .field {
  min-width: 12rem;
}

.filters .grow {
  flex: 1 1 16rem;
}

.case {
  font-weight: 620;
}

.small {
  font-size: var(--t-xs);
}
</style>
