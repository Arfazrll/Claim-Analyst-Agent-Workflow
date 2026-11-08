<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { api } from '@/api/client'
import type { EvidenceItem, PolicyChunk } from '@/api/types'
import TabStrip from '@/components/TabStrip.vue'

interface Dimension {
  key: string
  title: string
  query: string
}

const tab = ref('search')
const query = ref('room rent sub-limit per day')
const results = ref<EvidenceItem[]>([])
const searching = ref(false)
const chunks = ref<PolicyChunk[]>([])
const dimensions = ref<Dimension[]>([])
const synonyms = ref<[string, string][]>([])
const section = ref('')
const error = ref('')

const sections = computed(() => [...new Set(chunks.value.map((c) => c.section))])
const shown = computed(() => (section.value ? chunks.value.filter((c) => c.section === section.value) : chunks.value))

async function search() {
  if (!query.value.trim()) return
  searching.value = true
  error.value = ''
  try {
    results.value = (await api.get<{ results: EvidenceItem[] }>(`/api/policy/search?q=${encodeURIComponent(query.value)}&k=6`)).results
  } catch (e) {
    error.value = (e as Error).message
  } finally {
    searching.value = false
  }
}

onMounted(async () => {
  search()
  const [c, d] = await Promise.all([
    api.get<{ items: PolicyChunk[] }>('/api/policy/chunks'),
    api.get<{ items: Dimension[]; synonyms: [string, string][] }>('/api/policy/dimensions'),
  ])
  chunks.value = c.items
  dimensions.value = d.items
  synonyms.value = d.synonyms
})

function rank(v: number | null) {
  return v === null ? '—' : `#${v}`
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Policy wording</h1>
        <p>Universal Sompo CSC Individual Health Insurance, split into one clause per chunk. This is all the engine knows.</p>
      </div>
    </header>

    <TabStrip v-model="tab" :tabs="[{ id: 'search', label: 'Search' }, { id: 'browse', label: 'Browse', count: chunks.length }, { id: 'rules', label: 'Rules', count: dimensions.length }]" />

    <section v-if="tab === 'search'" id="panel-search" role="tabpanel" class="pane">
      <form class="searchbar" role="search" @submit.prevent="search">
        <label class="field grow"><span>Search the policy</span><input v-model="query" class="input" maxlength="300" /></label>
        <button class="btn btn-primary" type="submit" :disabled="searching">Search</button>
      </form>
      <p class="muted small">Hybrid retrieval: a TF-IDF vector ranking and a BM25 ranking are fused with reciprocal rank fusion, then reranked against the whole query. Each stage's rank is shown.</p>
      <p v-if="error" class="error-text">{{ error }}</p>
      <ol class="results">
        <li v-for="r in results" :key="r.chunk_id">
          <header>
            <h3>{{ r.label }}</h3>
            <span class="faint">page {{ r.page_start }} · <code>{{ r.chunk_id }}</code></span>
          </header>
          <p class="ranks faint">Vector {{ rank(r.dense_rank) }} · BM25 {{ rank(r.sparse_rank) }} · rerank score {{ r.rerank_score?.toFixed(3) }}</p>
          <p class="text">{{ r.text.length > 700 ? r.text.slice(0, 700) + ' …' : r.text }}</p>
        </li>
      </ol>
    </section>

    <section v-else-if="tab === 'browse'" id="panel-browse" role="tabpanel" class="pane">
      <label class="field narrow">
        <span>Section</span>
        <select v-model="section" class="select"><option value="">All sections</option><option v-for="s in sections" :key="s" :value="s">{{ s }}</option></select>
      </label>
      <ol class="results">
        <li v-for="c in shown" :key="c.chunk_id">
          <header><h3>{{ c.label }}</h3><span class="faint">page {{ c.page_start }} · <code>{{ c.chunk_id }}</code></span></header>
          <p class="text">{{ c.text }}</p>
        </li>
      </ol>
    </section>

    <section v-else id="panel-rules" role="tabpanel" class="pane">
      <p class="muted small">Each rule retrieves its clause, reads the number it needs from the retrieved text, and abstains if the number is not there.</p>
      <div class="table-wrap">
        <table class="table">
          <thead><tr><th>Rule</th><th>Retrieval query</th></tr></thead>
          <tbody><tr v-for="d in dimensions" :key="d.key"><td><strong>{{ d.title }}</strong></td><td class="muted">{{ d.query || 'diagnosis + procedure + "exclusion not covered"' }}</td></tr></tbody>
        </table>
      </div>
      <h2 class="syn-title">Clinical synonyms</h2>
      <p class="muted small">Case wording that maps to an item on the first-year waiting list. A synonym match only holds a claim for review; it never rejects one.</p>
      <ul class="syn">
        <li v-for="[a, b] in synonyms" :key="a"><span>{{ a }}</span><span class="faint">{{ b }}</span></li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.pane {
  padding-top: 1.5rem;
  display: grid;
  gap: 1rem;
}

.searchbar {
  display: flex;
  gap: 0.75rem;
  align-items: end;
  max-width: 48rem;
}

.grow {
  flex: 1;
}

.narrow {
  max-width: 20rem;
}

.small {
  font-size: var(--t-sm);
}

.results {
  list-style: none;
  margin: 0;
  padding: 0;
}

.results li {
  padding: 1rem 0;
  border-bottom: 1px solid var(--rule);
  display: grid;
  gap: 0.35rem;
}

.results header {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 1rem;
  align-items: baseline;
}

.results header span,
.ranks {
  font-size: var(--t-xs);
}

.text {
  font-size: var(--t-sm);
  max-width: 85ch;
}

.syn-title {
  margin-top: 1.5rem;
}

.syn {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
  gap: 0.3rem 2rem;
  font-size: var(--t-sm);
}

.syn li {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  border-bottom: 1px dashed var(--rule);
  padding: 0.3rem 0;
}
</style>
