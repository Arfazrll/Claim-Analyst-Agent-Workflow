<script setup lang="ts">
import type { Analysis } from '@/api/types'
import { AGENT_LABEL } from '@/utils/format'

defineProps<{ analysis: Analysis }>()
</script>

<template>
  <div class="slip">
    <p class="muted lead">The routing slip for this claim: each agent signs off what it read and wrote before handing the case on. A retry appears when validation sends the case back for wider retrieval.</p>
    <ol>
      <li v-for="(t, i) in analysis.trace" :key="i" :class="{ retry: t.attempt > 1 }">
        <div class="who">
          <span class="step">{{ i + 1 }}</span>
          <div>
            <h3>{{ AGENT_LABEL[t.agent] ?? t.agent }}</h3>
            <span class="faint">{{ t.action.replace(/_/g, ' ') }} · attempt {{ t.attempt }} · {{ t.elapsed_ms.toFixed(2) }} ms</span>
          </div>
        </div>
        <p>{{ t.detail }}</p>
        <p class="io faint">
          Read <span>{{ t.reads.join(', ') || '—' }}</span> · wrote <span>{{ t.writes.join(', ') || '—' }}</span>
          <template v-if="t.retrieval_count !== null"> · {{ t.retrieval_count }} chunks</template>
        </p>
      </li>
    </ol>
    <h3 class="handoffs-title">Hand-offs</h3>
    <ul class="handoffs">
      <li v-for="(h, i) in analysis.handoffs" :key="i" :data-kind="h.kind">
        <span class="route">{{ AGENT_LABEL[h.from_agent] ?? h.from_agent }} to {{ AGENT_LABEL[h.to_agent] ?? h.to_agent }}</span>
        <span class="muted">{{ h.payload }}</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.slip {
  display: grid;
  gap: 1rem;
}

.lead {
  font-size: var(--t-sm);
}

ol {
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid var(--rule-strong);
  border-radius: var(--r-md);
  background: var(--surface);
}

ol > li {
  padding: 1rem 1.25rem;
  border-bottom: 1px dashed var(--rule-strong);
  display: grid;
  gap: 0.4rem;
}

ol > li:last-child {
  border-bottom: 0;
}

ol > li.retry {
  background: color-mix(in srgb, var(--warn) 7%, transparent);
}

.who {
  display: flex;
  gap: 0.85rem;
  align-items: center;
}

.who span.faint {
  font-size: var(--t-xs);
}

.step {
  display: grid;
  place-items: center;
  width: 1.9rem;
  height: 1.9rem;
  border: 1.5px solid var(--ink-soft);
  border-radius: 50%;
  font-weight: 650;
  font-size: var(--t-xs);
  flex: none;
}

ol p {
  font-size: var(--t-sm);
  padding-left: 2.75rem;
}

.io {
  font-size: var(--t-xs) !important;
}

.handoffs-title {
  margin-top: 0.5rem;
}

.handoffs {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 0.4rem;
  font-size: var(--t-sm);
}

.handoffs li {
  display: grid;
  grid-template-columns: minmax(12rem, 18rem) 1fr;
  gap: 1rem;
}

.handoffs li[data-kind='retry'] .route {
  color: var(--warn);
  font-weight: 620;
}

@media (max-width: 640px) {
  .handoffs li {
    grid-template-columns: 1fr;
    gap: 0;
  }

  ol p {
    padding-left: 0;
  }
}
</style>
