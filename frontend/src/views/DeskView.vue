<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { api } from '@/api/client'
import type { ClaimSummary, Decision, Paged, Stats } from '@/api/types'
import DecisionTag from '@/components/DecisionTag.vue'
import EmptyState from '@/components/EmptyState.vue'
import { useAuth } from '@/stores/auth'
import { DECISIONS, DECISION_LABEL, DECISION_TONE, money, when } from '@/utils/format'

const auth = useAuth()
const stats = ref<Stats | null>(null)
const queue = ref<ClaimSummary[]>([])
const recent = ref<ClaimSummary[]>([])
const error = ref('')

onMounted(async () => {
  try {
    const [s, q, r] = await Promise.all([
      api.get<Stats>('/api/stats'),
      api.get<Paged<ClaimSummary>>('/api/claims?status=analyzed&page_size=8'),
      api.get<Paged<ClaimSummary>>('/api/claims?page_size=6'),
    ])
    stats.value = s
    queue.value = q.items
    recent.value = r.items
  } catch (e) {
    error.value = (e as Error).message
  }
})

const total = computed(() => stats.value?.stats.total_claims ?? 0)
const counts = computed(() => {
  const m = new Map<Decision, number>(stats.value?.stats.by_decision ?? [])
  return DECISIONS.map((d) => ({ d, n: m.get(d) ?? 0 }))
})
const anchored = computed(() => {
  const s = stats.value?.stats
  if (!s) return null
  const by = new Map(s.ledger_by_status)
  return { total: s.ledger_entries, settled: (by.get('anchored') ?? 0) + (by.get('pinned') ?? 0) + (by.get('local') ?? 0), waiting: (by.get('pending') ?? 0) + (by.get('retry') ?? 0), failed: by.get('failed') ?? 0 }
})
const greeting = computed(() => {
  const h = new Date().getHours()
  return h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening'
})
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>{{ greeting }}, {{ auth.user?.name.split(' ')[0] }}</h1>
        <p>{{ stats?.scope === 'all' ? 'Every claim on the desk.' : 'Claims you have submitted.' }}</p>
      </div>
      <RouterLink to="/claims/new" class="btn btn-primary">Analyse a claim</RouterLink>
    </header>

    <p v-if="error" class="error-text">{{ error }}</p>

    <section v-if="stats" class="docket" aria-label="Decision mix">
      <div class="figures">
        <div><span class="fig">{{ total }}</span><span class="muted">claims analysed</span></div>
        <div><span class="fig">{{ money(stats.stats.claimed_total) }}</span><span class="muted">claimed</span></div>
        <div><span class="fig">{{ money(stats.stats.payable_total) }}</span><span class="muted">estimated payable</span></div>
      </div>
      <div v-if="total" class="mix" role="img" :aria-label="counts.map((c) => `${DECISION_LABEL[c.d]} ${c.n}`).join(', ')">
        <span v-for="c in counts.filter((c) => c.n)" :key="c.d" :style="{ flexGrow: c.n, background: DECISION_TONE[c.d] }" :title="`${DECISION_LABEL[c.d]}: ${c.n}`"></span>
      </div>
      <ul v-if="total" class="legend">
        <li v-for="c in counts" :key="c.d"><DecisionTag :decision="c.d" /> <strong>{{ c.n }}</strong></li>
      </ul>
      <p v-if="anchored && anchored.total" class="ledger-line muted">
        Ledger: {{ anchored.settled }} of {{ anchored.total }} events sealed<template v-if="anchored.waiting">, {{ anchored.waiting }} waiting to anchor</template><template v-if="anchored.failed">, <span class="error-text">{{ anchored.failed }} failed</span></template>.
      </p>
    </section>

    <div class="cols">
      <section>
        <h2>Waiting for sign-off</h2>
        <p class="muted sub">Analysed by the engine, not yet reviewed.</p>
        <EmptyState v-if="!queue.length" title="Nothing waiting" body="Every analysed claim has been reviewed." />
        <ul v-else class="list">
          <li v-for="c in queue" :key="c.id">
            <RouterLink :to="`/claims/${c.id}`">{{ c.case_id }}</RouterLink>
            <DecisionTag :decision="c.decision" />
            <span class="faint">{{ when(c.created_at) }}</span>
          </li>
        </ul>
      </section>
      <section>
        <h2>Recent</h2>
        <p class="muted sub">Latest submissions.</p>
        <EmptyState v-if="!recent.length" title="No claims yet" body="Analyse a sample case to see the full workflow.">
          <RouterLink to="/claims/new" class="btn">Open a sample case</RouterLink>
        </EmptyState>
        <ul v-else class="list">
          <li v-for="c in recent" :key="c.id">
            <RouterLink :to="`/claims/${c.id}`">{{ c.case_id }}</RouterLink>
            <span>{{ money(c.payable) }}</span>
            <span class="faint">{{ c.created_by_name }}</span>
          </li>
        </ul>
      </section>
    </div>
  </div>
</template>

<style scoped>
.docket {
  padding: 1.5rem 0 2rem;
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule);
  margin-bottom: 2.25rem;
  display: grid;
  gap: 1.25rem;
}

.figures {
  display: flex;
  flex-wrap: wrap;
  gap: 1.5rem 3rem;
}

.figures div {
  display: grid;
}

.fig {
  font-size: var(--t-2xl);
  font-weight: 650;
  font-stretch: 112%;
  line-height: 1.1;
}

.figures .muted {
  font-size: var(--t-sm);
}

.mix {
  display: flex;
  height: 0.9rem;
  border-radius: 2px;
  overflow: hidden;
  gap: 2px;
}

.mix span {
  min-width: 4px;
}

.legend {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem 1.5rem;
  font-size: var(--t-sm);
}

.legend li {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.ledger-line {
  font-size: var(--t-sm);
}

.cols {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 2.5rem;
}

.sub {
  font-size: var(--t-sm);
  margin: 0.2rem 0 1rem;
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.list li {
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 1rem;
  align-items: baseline;
  padding: 0.6rem 0;
  border-bottom: 1px solid var(--rule);
  font-size: var(--t-sm);
}

.list a {
  font-weight: 600;
}

.list .faint {
  font-size: var(--t-xs);
  min-width: 7rem;
  text-align: right;
}
</style>
