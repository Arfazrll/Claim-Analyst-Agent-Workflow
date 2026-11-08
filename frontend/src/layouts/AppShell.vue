<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuth } from '@/stores/auth'
import { ROLE_LABEL } from '@/utils/format'

const auth = useAuth()
const router = useRouter()
const route = useRoute()
const open = ref(false)

watch(() => route.fullPath, () => (open.value = false))

const links = computed(() =>
  [
    { to: '/', label: 'Desk', show: true },
    { to: '/claims', label: 'Claims', show: true },
    { to: '/claims/new', label: 'New claim', show: true },
    { to: '/policy', label: 'Policy wording', show: true },
    { to: '/ledger', label: 'Ledger', show: auth.isReviewer },
    { to: '/evaluation', label: 'Evaluation', show: auth.isReviewer },
    { to: '/admin', label: 'Administration', show: auth.isAdmin },
  ].filter((l) => l.show),
)

function toggleTheme() {
  const root = document.documentElement
  const dark = root.dataset.theme === 'dark' || (!root.dataset.theme && matchMedia('(prefers-color-scheme: dark)').matches)
  root.dataset.theme = dark ? 'light' : 'dark'
  try {
    localStorage.setItem('theme', root.dataset.theme)
  } catch {
    /* ignore */
  }
}

async function signOut() {
  await auth.logout()
  router.push({ name: 'login' })
}
</script>

<template>
  <div class="shell">
    <header class="bar">
      <button class="btn btn-quiet menu" type="button" :aria-expanded="open" aria-controls="rail" @click="open = !open">Menu</button>
      <RouterLink to="/" class="mark">Claim Desk</RouterLink>
    </header>
    <nav id="rail" class="rail" :class="{ open }" aria-label="Main">
      <RouterLink to="/" class="mark rail-mark">Claim Desk</RouterLink>
      <ul>
        <li v-for="l in links" :key="l.to">
          <RouterLink :to="l.to" :class="{ exact: l.to === '/' || l.to === '/claims' }">{{ l.label }}</RouterLink>
        </li>
      </ul>
      <div class="who">
        <RouterLink to="/account" class="who-name">{{ auth.user?.name }}</RouterLink>
        <span class="faint">{{ auth.user ? ROLE_LABEL[auth.user.role] : '' }}</span>
        <div class="who-actions">
          <button class="btn btn-quiet" type="button" @click="toggleTheme">Theme</button>
          <button class="btn btn-quiet" type="button" @click="signOut">Sign out</button>
        </div>
      </div>
    </nav>
    <main class="main">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: grid;
  grid-template-columns: var(--rail) 1fr;
  min-height: 100vh;
}

.bar {
  display: none;
}

.rail {
  position: sticky;
  top: 0;
  height: 100vh;
  display: flex;
  flex-direction: column;
  padding: 1.5rem 1rem;
  border-right: 1px solid var(--rule);
  background: var(--surface);
}

.mark {
  font-stretch: 125%;
  font-weight: 720;
  font-size: var(--t-lg);
  color: var(--ink);
  text-decoration: none;
  letter-spacing: -0.02em;
}

.rail-mark {
  padding: 0 0.6rem 1.5rem;
}

.rail ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 2px;
}

.rail a:not(.mark):not(.who-name) {
  display: block;
  padding: 0.5rem 0.6rem;
  border-radius: var(--r-sm);
  color: var(--ink-soft);
  text-decoration: none;
  font-size: var(--t-sm);
  font-weight: 540;
}

.rail a:not(.mark):not(.who-name):hover {
  background: var(--surface-sunk);
  color: var(--ink);
}

/* Desk and Claims match exactly so "New claim" is not double-highlighted. */
.rail li a.router-link-active:not(.exact),
.rail li a.exact.router-link-exact-active {
  background: var(--surface-sunk);
  color: var(--ink);
  box-shadow: inset 3px 0 0 var(--petrol);
}

.who {
  margin-top: auto;
  display: grid;
  gap: 0.15rem;
  padding: 1rem 0.6rem 0;
  border-top: 1px solid var(--rule);
  font-size: var(--t-sm);
}

.who-name {
  color: var(--ink);
  font-weight: 600;
  text-decoration: none;
}

.who-actions {
  display: flex;
  gap: 0.25rem;
  margin: 0.5rem -0.6rem 0;
}

.main {
  min-width: 0;
}

@media (max-width: 860px) {
  .shell {
    grid-template-columns: 1fr;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--rule);
    background: var(--surface);
    position: sticky;
    top: 0;
    z-index: 20;
  }

  .rail {
    display: none;
    position: fixed;
    inset: 3.4rem 0 0 0;
    height: auto;
    z-index: 19;
    border-right: 0;
  }

  .rail.open {
    display: flex;
  }

  .rail-mark {
    display: none;
  }
}
</style>
