import { defineStore } from 'pinia'
import { ref } from 'vue'

export interface Toast {
  id: number
  text: string
  tone: 'ok' | 'bad' | 'info'
}

export const useToasts = defineStore('toasts', () => {
  const items = ref<Toast[]>([])
  let next = 1

  function push(text: string, tone: Toast['tone'] = 'info', ms = 4500) {
    const id = next++
    items.value.push({ id, text, tone })
    window.setTimeout(() => dismiss(id), ms)
  }

  function dismiss(id: number) {
    items.value = items.value.filter((t) => t.id !== id)
  }

  return { items, push, dismiss }
})
