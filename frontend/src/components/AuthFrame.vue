<script setup lang="ts">
defineProps<{ title: string; lead: string }>()
</script>

<template>
  <div class="frame">
    <section class="intro" aria-hidden="true">
      <p class="mark">Claim Desk</p>
      <div class="slip">
        <p class="slip-head">Routing slip</p>
        <ol>
          <li><span>Case analysis</span><span>facts &amp; checks</span></li>
          <li><span>Policy evidence</span><span>hybrid search</span></li>
          <li><span>Coverage &amp; exclusion</span><span>11 rules</span></li>
          <li><span>Decision</span><span>precedence</span></li>
          <li><span>Validation</span><span>every citation</span></li>
        </ol>
        <p class="slip-foot">Each decision is cited to the policy wording and written to a tamper-evident ledger.</p>
      </div>
    </section>
    <section class="form-side">
      <div class="form-box">
        <h1>{{ title }}</h1>
        <p class="muted">{{ lead }}</p>
        <slot />
      </div>
    </section>
  </div>
</template>

<style scoped>
.frame {
  min-height: 100vh;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr);
}

.intro {
  background: var(--ink);
  color: var(--paper);
  padding: 2.5rem clamp(1.5rem, 4vw, 4rem);
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 2rem;
}

.mark {
  font-stretch: 125%;
  font-weight: 720;
  font-size: var(--t-xl);
}

.slip {
  border: 1px solid color-mix(in srgb, var(--paper) 30%, transparent);
  border-radius: var(--r-md);
  padding: 1.25rem 1.5rem;
  max-width: 26rem;
  transform: rotate(-1.5deg);
}

.slip-head {
  font-stretch: 112%;
  font-weight: 650;
  margin-bottom: 0.75rem;
}

ol {
  margin: 0;
  padding: 0;
  list-style: none;
  counter-reset: step;
}

li {
  counter-increment: step;
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.55rem 0;
  border-bottom: 1px dashed color-mix(in srgb, var(--paper) 25%, transparent);
  font-size: var(--t-sm);
}

li span:first-child::before {
  content: counter(step) '.';
  display: inline-block;
  width: 1.4rem;
  opacity: 0.6;
}

li span:last-child {
  opacity: 0.65;
}

.slip-foot {
  margin-top: 1rem;
  font-size: var(--t-xs);
  opacity: 0.75;
}

.form-side {
  display: grid;
  place-items: center;
  padding: 2rem 1rem;
}

.form-box {
  width: min(100%, 24rem);
  display: grid;
  gap: 0.5rem;
}

.form-box h1 {
  font-size: var(--t-xl);
}

@media (max-width: 860px) {
  .frame {
    grid-template-columns: 1fr;
  }

  .intro {
    padding: 1.25rem 1rem;
  }

  .slip {
    display: none;
  }
}
</style>
