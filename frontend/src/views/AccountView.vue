<script setup lang="ts">
import { ref } from 'vue'
import { api, ApiError } from '@/api/client'
import { useAuth } from '@/stores/auth'
import { useToasts } from '@/stores/toast'
import { ROLE_LABEL, when } from '@/utils/format'

const auth = useAuth()
const toasts = useToasts()
const current = ref('')
const next = ref('')
const error = ref('')
const busy = ref(false)

async function change() {
  busy.value = true
  error.value = ''
  try {
    await api.post('/api/auth/password', { current_password: current.value, new_password: next.value })
    current.value = next.value = ''
    toasts.push('Password changed. Other sessions were signed out.', 'ok')
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'The password could not be changed.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head"><div><h1>Account</h1></div></header>
    <dl v-if="auth.user" class="who">
      <dt>Name</dt><dd>{{ auth.user.name }}</dd>
      <dt>Email</dt><dd>{{ auth.user.email }}</dd>
      <dt>Role</dt><dd>{{ ROLE_LABEL[auth.user.role] }}</dd>
      <dt>Member since</dt><dd>{{ when(auth.user.created_at) }}</dd>
    </dl>
    <form class="form" @submit.prevent="change">
      <h2>Change password</h2>
      <label class="field"><span>Current password</span><input v-model="current" class="input" type="password" autocomplete="current-password" required /></label>
      <label class="field"><span>New password</span><input v-model="next" class="input" type="password" autocomplete="new-password" required minlength="12" /><small>At least 12 characters, mixing three kinds of character.</small></label>
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <div><button class="btn btn-primary" type="submit" :disabled="busy">Change password</button></div>
    </form>
  </div>
</template>

<style scoped>
.who {
  display: grid;
  grid-template-columns: 9rem 1fr;
  gap: 0.5rem 1rem;
  margin: 0 0 2.5rem;
  font-size: var(--t-sm);
}

.who dt {
  color: var(--ink-faint);
}

.who dd {
  margin: 0;
}

.form {
  display: grid;
  gap: 1rem;
  max-width: 26rem;
}
</style>
