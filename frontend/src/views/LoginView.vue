<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ApiError } from '@/api/client'
import AuthFrame from '@/components/AuthFrame.vue'
import { useAuth } from '@/stores/auth'

const auth = useAuth()
const router = useRouter()
const route = useRoute()
const email = ref('')
const password = ref('')
const error = ref(route.query.expired ? 'Your session ended. Sign in again to continue.' : '')
const busy = ref(false)

function safeNext(): string {
  const n = typeof route.query.next === 'string' ? route.query.next : '/'
  // Only same-app paths: never redirect to another origin.
  return n.startsWith('/') && !n.startsWith('//') ? n : '/'
}

async function submit() {
  busy.value = true
  error.value = ''
  try {
    await auth.login(email.value, password.value)
    router.replace(safeNext())
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Sign-in failed. Check your connection and try again.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AuthFrame title="Sign in" lead="Analyse claims against the policy wording and sign off decisions.">
    <form class="form" @submit.prevent="submit">
      <label class="field"><span>Email</span><input v-model="email" class="input" type="email" autocomplete="username" required /></label>
      <label class="field"><span>Password</span><input v-model="password" class="input" type="password" autocomplete="current-password" required /></label>
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <button class="btn btn-primary" type="submit" :disabled="busy">{{ busy ? 'Signing in…' : 'Sign in' }}</button>
      <p class="muted small">No account? <RouterLink to="/register">Create one</RouterLink></p>
    </form>
  </AuthFrame>
</template>

<style scoped>
.form {
  display: grid;
  gap: 1rem;
  margin-top: 1rem;
}

.small {
  font-size: var(--t-sm);
}
</style>
