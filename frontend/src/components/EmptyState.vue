<script setup lang="ts">
defineProps<{ title: string; body?: string }>()
</script>

<template>
  <div class="empty">
    <h3>{{ title }}</h3>
    <p v-if="body" class="muted">{{ body }}</p>
    <div class="actions"><slot /></div>
  </div>
</template>

<style scoped>
.empty {
  display: grid;
  gap: 0.5rem;
  justify-items: start;
  padding: 2rem 1.5rem;
  border: 1px dashed var(--rule-strong);
  border-radius: var(--r-md);
}

.actions:empty {
  display: none;
}

.actions {
  margin-top: 0.5rem;
  display: flex;
  gap: 0.5rem;
}
</style>
