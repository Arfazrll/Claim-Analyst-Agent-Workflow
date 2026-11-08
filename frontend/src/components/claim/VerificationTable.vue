<script setup lang="ts">
import { ref } from 'vue'
import type { Analysis, Verdict } from '@/api/types'
import { humanize } from '@/utils/format'

defineProps<{ validation: Analysis['validation'] }>()
const open = ref<string | null>(null)

const TONE: Record<Verdict, string> = { SUPPORTED: 'var(--ok)', UNSUPPORTED: 'var(--warn)', CONTRADICTED: 'var(--bad)' }
const LABEL: Record<Verdict, string> = { SUPPORTED: 'Supported', UNSUPPORTED: 'Unsupported', CONTRADICTED: 'Contradicted' }
</script>

<template>
  <div class="stack">
    <p class="summary">
      <strong :style="{ color: validation.status === 'PASS' ? 'var(--ok)' : 'var(--bad)' }">
        {{ validation.status === 'PASS' ? 'All claims verified' : 'Verification failed' }}
      </strong>
      <span class="muted"> — {{ validation.notes }} Attempts: {{ validation.attempts }}.</span>
    </p>
    <div class="table-wrap">
      <table class="table">
        <thead>
          <tr><th>Claim</th><th>Rule</th><th>Cited chunk</th><th>Verdict</th><th></th></tr>
        </thead>
        <tbody>
          <template v-for="v in validation.verifications" :key="v.claim_id">
            <tr>
              <td>{{ v.claim }}</td>
              <td class="faint">{{ humanize(v.dimension) }}</td>
              <td><code v-if="v.chunk_id">{{ v.chunk_id }}</code><span v-else class="faint">decision</span></td>
              <td :style="{ color: TONE[v.verdict] }"><strong>{{ LABEL[v.verdict] }}</strong></td>
              <td>
                <button class="btn btn-quiet" type="button" :aria-expanded="open === v.claim_id" @click="open = open === v.claim_id ? null : v.claim_id">
                  {{ open === v.claim_id ? 'Hide checks' : 'Checks' }}
                </button>
              </td>
            </tr>
            <tr v-if="open === v.claim_id" class="detail">
              <td colspan="5">
                <p class="muted">{{ v.reason }}</p>
                <ul>
                  <li v-for="(c, i) in v.checks" :key="i" :class="{ fail: !c.passed }">
                    {{ c.passed ? 'Passed' : 'Failed' }}: {{ c.name }}<span v-if="c.detail" class="faint"> — {{ c.detail }}</span>
                  </li>
                </ul>
                <blockquote v-if="v.excerpt">{{ v.excerpt }}</blockquote>
              </td>
            </tr>
          </template>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.stack {
  display: grid;
  gap: 1rem;
}

.summary {
  font-size: var(--t-sm);
}

.detail td {
  background: var(--surface-sunk);
}

.detail ul {
  margin: 0.5rem 0;
  padding-left: 1.1rem;
  font-size: var(--t-xs);
  display: grid;
  gap: 0.2rem;
}

.detail .fail {
  color: var(--bad);
}

blockquote {
  margin: 0.5rem 0 0;
  font-size: var(--t-xs);
  border-left: 2px solid var(--rule-strong);
  padding-left: 0.75rem;
}
</style>
