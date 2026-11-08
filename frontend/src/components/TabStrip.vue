<script setup lang="ts">
const props = defineProps<{ tabs: { id: string; label: string; count?: number }[] }>()
const model = defineModel<string>({ required: true })

function onKey(e: KeyboardEvent) {
  const i = props.tabs.findIndex((t) => t.id === model.value)
  const delta = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0
  if (!delta) return
  e.preventDefault()
  const next = props.tabs[(i + delta + props.tabs.length) % props.tabs.length]
  if (next) {
    model.value = next.id
    ;(document.getElementById(`tab-${next.id}`) as HTMLElement | null)?.focus()
  }
}
</script>

<template>
  <div class="tabs" role="tablist" @keydown="onKey">
    <button
      v-for="t in tabs"
      :id="`tab-${t.id}`"
      :key="t.id"
      role="tab"
      type="button"
      :aria-selected="model === t.id"
      :tabindex="model === t.id ? 0 : -1"
      :aria-controls="`panel-${t.id}`"
      @click="model = t.id"
    >
      {{ t.label }}<span v-if="t.count !== undefined" class="count">{{ t.count }}</span>
    </button>
  </div>
</template>

<style scoped>
.tabs {
  display: flex;
  gap: 0.25rem;
  border-bottom: 1px solid var(--rule-strong);
  overflow-x: auto;
  scrollbar-width: thin;
}

button {
  font: inherit;
  font-size: var(--t-sm);
  font-weight: 560;
  color: var(--ink-soft);
  background: none;
  border: 0;
  padding: 0.65rem 0.8rem;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  cursor: pointer;
  white-space: nowrap;
}

button[aria-selected='true'] {
  color: var(--ink);
  border-bottom-color: var(--petrol);
}

.count {
  margin-left: 0.4rem;
  color: var(--ink-faint);
  font-weight: 500;
}
</style>
