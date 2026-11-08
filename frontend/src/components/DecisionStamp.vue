<script setup lang="ts">
import { computed } from 'vue'
import type { Decision } from '@/api/types'
import { DECISION_LABEL, DECISION_STAMP, DECISION_TONE } from '@/utils/format'

const props = withDefaults(defineProps<{ decision: Decision; press?: boolean; caption?: string }>(), { press: false, caption: '' })
const tone = computed(() => DECISION_TONE[props.decision])
</script>

<template>
  <div class="stamp" :class="{ press }" :style="{ '--tone': tone }" role="img" :aria-label="`Decision: ${DECISION_LABEL[decision]}`">
    <span class="word">{{ DECISION_STAMP[decision] }}</span>
    <span v-if="caption" class="caption">{{ caption }}</span>
  </div>
</template>

<style scoped>
/* The one loud element on the page: a rubber stamp in the decision's ink. */
.stamp {
  display: inline-grid;
  justify-items: center;
  gap: 0.15rem;
  padding: 0.55rem 1.1rem 0.5rem;
  border: 3px solid var(--tone);
  outline: 1.5px solid var(--tone);
  outline-offset: 3px;
  border-radius: 6px;
  color: var(--tone);
  transform: rotate(-4deg);
  background:
    radial-gradient(circle at 20% 30%, color-mix(in srgb, var(--tone) 9%, transparent) 0 30%, transparent 31%),
    radial-gradient(circle at 75% 70%, color-mix(in srgb, var(--tone) 7%, transparent) 0 25%, transparent 26%);
  mix-blend-mode: multiply;
}

:root[data-theme='dark'] .stamp {
  mix-blend-mode: normal;
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme='light']) .stamp {
    mix-blend-mode: normal;
  }
}

.word {
  font-stretch: 75%;
  font-weight: 800;
  font-size: clamp(1.5rem, 3.4vw, 2.2rem);
  line-height: 1;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.caption {
  font-size: var(--t-xs);
  font-weight: 600;
  font-stretch: 90%;
  letter-spacing: 0.02em;
}

.press {
  animation: press 520ms cubic-bezier(0.2, 0.9, 0.25, 1.15) both;
}

@keyframes press {
  0% {
    opacity: 0;
    transform: rotate(-9deg) scale(1.45);
  }
  60% {
    opacity: 1;
    transform: rotate(-3deg) scale(0.96);
  }
  100% {
    transform: rotate(-4deg) scale(1);
  }
}
</style>
