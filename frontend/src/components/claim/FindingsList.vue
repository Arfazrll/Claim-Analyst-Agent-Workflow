<script setup lang="ts">
import { computed } from 'vue'
import type { Analysis, FindingStatus } from '@/api/types'
import { STATUS_LABEL, humanize, money } from '@/utils/format'

const props = defineProps<{ analysis: Analysis }>()

const TONE: Record<FindingStatus, string> = {
  SUPPORTS_ADMISSIBLE: 'var(--ok)',
  SUPPORTS_LIMIT: 'var(--warn)',
  SUPPORTS_EXCLUSION: 'var(--bad)',
  INSUFFICIENT_EVIDENCE: 'var(--d-review)',
  NOT_APPLICABLE: 'var(--ink-faint)',
}

// Most consequential first: exclusions, unresolved, limits, satisfied, untriggered.
const ORDER: FindingStatus[] = ['SUPPORTS_EXCLUSION', 'INSUFFICIENT_EVIDENCE', 'SUPPORTS_LIMIT', 'SUPPORTS_ADMISSIBLE', 'NOT_APPLICABLE']
const findings = computed(() => [...props.analysis.findings].sort((a, b) => ORDER.indexOf(a.status) - ORDER.indexOf(b.status)))
</script>

<template>
  <div class="stack">
    <section v-if="analysis.missing_evidence.length" class="attention">
      <h3>Needs attention</h3>
      <ul>
        <li v-for="(m, i) in analysis.missing_evidence" :key="i">
          <strong>{{ humanize(m.field) }}.</strong> {{ m.reason }}
        </li>
      </ul>
    </section>

    <ol class="findings">
      <li v-for="f in findings" :key="f.dimension" :style="{ '--tone': TONE[f.status] }">
        <div class="f-head">
          <span class="f-status">{{ STATUS_LABEL[f.status] }}</span>
          <h3>{{ humanize(f.dimension) }}</h3>
          <span v-if="f.source === 'llm'" class="faint">model-suggested</span>
          <span class="faint conf">confidence {{ Math.round(f.confidence * 100) }}%</span>
        </div>
        <p>{{ f.statement }}</p>
        <p v-if="f.chunk_ids.length" class="faint cites">Cites {{ f.chunk_ids.join(', ') }}</p>
        <ul v-if="f.assumptions.length" class="assume">
          <li v-for="(a, i) in f.assumptions" :key="i">Assumption: {{ a }}</li>
        </ul>
      </li>
    </ol>

    <section v-if="analysis.applicable_limits.length">
      <h3>Deductions</h3>
      <div class="table-wrap">
        <table class="table">
          <thead><tr><th>Limit</th><th class="num">Deduction</th></tr></thead>
          <tbody>
            <tr v-for="(l, i) in analysis.applicable_limits" :key="i">
              <td>{{ l.description }}</td>
              <td class="num">{{ money(l.deduction_inr) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <section>
      <h3>Rationale</h3>
      <p class="rationale">{{ analysis.rationale }}</p>
      <p class="faint small">Written by: {{ analysis.rationale_source }}</p>
    </section>
  </div>
</template>

<style scoped>
.stack {
  display: grid;
  gap: 1.75rem;
}

.attention {
  border-left: 3px solid var(--d-review);
  padding: 0.25rem 0 0.25rem 1rem;
}

.attention ul {
  margin: 0.5rem 0 0;
  padding-left: 1.1rem;
  display: grid;
  gap: 0.4rem;
  font-size: var(--t-sm);
}

.findings {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
}

.findings > li {
  padding: 1rem 0 1rem 1rem;
  border-bottom: 1px solid var(--rule);
  border-left: 3px solid var(--tone);
  display: grid;
  gap: 0.35rem;
}

.f-head {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.25rem 0.75rem;
}

.f-status {
  color: var(--tone);
  font-weight: 650;
  font-size: var(--t-xs);
  min-width: 6.5rem;
}

.conf {
  margin-left: auto;
  font-size: var(--t-xs);
}

.findings p {
  font-size: var(--t-sm);
}

.cites,
.small {
  font-size: var(--t-xs);
}

.assume {
  margin: 0.2rem 0 0;
  padding-left: 1rem;
  font-size: var(--t-xs);
  color: var(--ink-soft);
}

.rationale {
  white-space: pre-line;
  font-size: var(--t-sm);
  margin: 0.5rem 0 0.35rem;
}
</style>
