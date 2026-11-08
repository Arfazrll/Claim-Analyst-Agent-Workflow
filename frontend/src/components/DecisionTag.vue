<script setup lang="ts">
import type { Decision } from '@/api/types'
import { DECISION_LABEL, DECISION_TONE } from '@/utils/format'

defineProps<{ decision: Decision }>()
</script>

<template>
  <span class="tag" :style="{ '--tone': DECISION_TONE[decision] }">{{ DECISION_LABEL[decision] }}</span>
</template>

<style scoped>
.tag {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-size: var(--t-xs);
  font-weight: 620;
  color: var(--tone);
  white-space: nowrap;
}

.tag::before {
  content: '';
  width: 0.55rem;
  height: 0.55rem;
  border: 2px solid var(--tone);
  border-radius: 2px;
  transform: rotate(-8deg);
}
</style>
