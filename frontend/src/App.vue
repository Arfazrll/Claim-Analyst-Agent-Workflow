<script setup lang="ts">
import { useToasts } from '@/stores/toast'

const toasts = useToasts()
</script>

<template>
  <RouterView />
  <div class="toasts" role="status" aria-live="polite">
    <button v-for="t in toasts.items" :key="t.id" class="toast" :data-tone="t.tone" type="button" @click="toasts.dismiss(t.id)">
      {{ t.text }}
    </button>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  right: 1rem;
  bottom: 1rem;
  display: grid;
  gap: 0.5rem;
  z-index: 50;
  max-width: min(420px, calc(100vw - 2rem));
}

.toast {
  text-align: left;
  font: inherit;
  font-size: var(--t-sm);
  padding: 0.75rem 1rem;
  border-radius: var(--r-sm);
  border: 1px solid var(--rule-strong);
  border-left: 4px solid var(--petrol);
  background: var(--surface);
  color: var(--ink);
  box-shadow: var(--shadow-pop);
  cursor: pointer;
}

.toast[data-tone='ok'] {
  border-left-color: var(--ok);
}

.toast[data-tone='bad'] {
  border-left-color: var(--bad);
}
</style>
