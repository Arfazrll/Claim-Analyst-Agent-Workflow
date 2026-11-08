<script setup lang="ts">
import type { Citation } from '@/api/types'

defineProps<{ citations: Citation[] }>()
</script>

<template>
  <div class="cites">
    <p class="muted lead">Every statement in the decision points to a clause of the policy wording. The excerpt is the retrieved text the rule read its numbers from.</p>
    <article v-for="c in citations" :key="`${c.chunk_id}-${c.claim}`" class="cite">
      <header>
        <h3>{{ c.claim }}</h3>
        <span class="faint">{{ c.section }}, page {{ c.page }} · <code>{{ c.chunk_id }}</code></span>
      </header>
      <blockquote v-if="c.excerpt">{{ c.excerpt }}</blockquote>
    </article>
  </div>
</template>

<style scoped>
.cites {
  display: grid;
  gap: 1.25rem;
}

.lead {
  font-size: var(--t-sm);
}

.cite header {
  display: grid;
  gap: 0.15rem;
}

.cite header span {
  font-size: var(--t-xs);
}

blockquote {
  margin: 0.6rem 0 0;
  padding: 0.75rem 1rem;
  background: var(--surface-sunk);
  border-radius: var(--r-sm);
  font-size: var(--t-sm);
  line-height: 1.6;
  max-width: 80ch;
}
</style>
