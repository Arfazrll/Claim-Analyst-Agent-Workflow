<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ page: number; pageSize: number; total: number }>()
const emit = defineEmits<{ go: [page: number] }>()
const pages = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)))
</script>

<template>
  <nav v-if="pages > 1" class="pager" aria-label="Pagination">
    <button class="btn" type="button" :disabled="page <= 1" @click="emit('go', page - 1)">Previous</button>
    <span class="muted">Page {{ page }} of {{ pages }}</span>
    <button class="btn" type="button" :disabled="page >= pages" @click="emit('go', page + 1)">Next</button>
  </nav>
</template>

<style scoped>
.pager {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-top: 1rem;
  font-size: var(--t-sm);
}
</style>
