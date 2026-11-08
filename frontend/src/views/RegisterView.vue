<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { ApiError } from '@/api/client'
import AuthFrame from '@/components/AuthFrame.vue'
import { useAuth } from '@/stores/auth'

const auth = useAuth()
const router = useRouter()
const name = ref('')
const email = ref('')
const password = ref('')
const error = ref('')
const busy = ref(false)

const strength = computed(() => {
  const p = password.value
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length
  if (p.length < 12) return { ok: false, text: `${12 - p.length} more character(s) needed` }
  if (classes < 3) return { ok: false, text: 'Mix at least three of: lowercase, uppercase, digits, symbols' }
  return { ok: true, text: 'Strong enough' }
})

async function submit() {
  busy.value = true
  error.value = ''
  try {
    await auth.register(name.value, email.value, password.value)
    router.replace('/')
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'The account could not be created.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AuthFrame title="Create account" lead="New accounts start as analysts. An administrator can grant reviewer access.">
    <form class="form" @submit.prevent="submit">
      <label class="field"><span>Name</span><input v-model="name" class="input" autocomplete="name" required minlength="2" maxlength="80" /></label>
      <label class="field"><span>Email</span><input v-model="email" class="input" type="email" autocomplete="email" required /></label>
      <label class="field">
        <span>Password</span>
        <input v-model="password" class="input" type="password" autocomplete="new-password" required minlength="12" />
        <small :style="{ color: password && strength.ok ? 'var(--ok)' : undefined }">{{ password ? strength.text : 'At least 12 characters' }}</small>
      </label>
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <button class="btn btn-primary" type="submit" :disabled="busy || !strength.ok">{{ busy ? 'Creating…' : 'Create account' }}</button>
      <p class="muted small">Already registered? <RouterLink to="/login">Sign in</RouterLink></p>
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
