<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api, ApiError } from '@/api/client'
import type { Paged, Role, User } from '@/api/types'
import PagerNav from '@/components/PagerNav.vue'
import TabStrip from '@/components/TabStrip.vue'
import { useAuth } from '@/stores/auth'
import { useToasts } from '@/stores/toast'
import { ROLE_LABEL, when } from '@/utils/format'

interface AuditRow {
  id: string
  actor_email: string | null
  action: string
  target: string | null
  ip: string | null
  detail: string | null
  created_at: number
}

const auth = useAuth()
const toasts = useToasts()
const tab = ref('users')
const users = ref<User[]>([])
const audit = ref<Paged<AuditRow> | null>(null)
const page = ref(1)

async function loadUsers() {
  users.value = (await api.get<{ items: User[] }>('/api/admin/users')).items
}
async function loadAudit() {
  audit.value = await api.get<Paged<AuditRow>>(`/api/admin/audit?page=${page.value}&page_size=30`)
}
onMounted(() => Promise.all([loadUsers(), loadAudit()]))

async function update(u: User, patch: { role?: Role; active?: boolean }) {
  try {
    await api.patch(`/api/admin/users/${u.id}`, patch)
    toasts.push(`Updated ${u.email}.`, 'ok')
  } catch (e) {
    toasts.push(e instanceof ApiError ? e.message : 'Update failed.', 'bad')
  }
  await Promise.all([loadUsers(), loadAudit()])
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Administration</h1>
        <p>Grant roles and read the security audit trail.</p>
      </div>
    </header>
    <TabStrip v-model="tab" :tabs="[{ id: 'users', label: 'People', count: users.length }, { id: 'audit', label: 'Audit log', count: audit?.total }]" />

    <div v-if="tab === 'users'" id="panel-users" role="tabpanel" class="table-wrap pane">
      <table class="table">
        <thead><tr><th>Name</th><th>Email</th><th>Role</th><th>Access</th><th>Joined</th></tr></thead>
        <tbody>
          <tr v-for="u in users" :key="u.id">
            <td><strong>{{ u.name }}</strong></td>
            <td>{{ u.email }}</td>
            <td>
              <select class="select" :value="u.role" :disabled="u.id === auth.user?.id" :aria-label="`Role for ${u.email}`" @change="update(u, { role: ($event.target as HTMLSelectElement).value as Role })">
                <option v-for="(label, r) in ROLE_LABEL" :key="r" :value="r">{{ label }}</option>
              </select>
            </td>
            <td>
              <button class="btn" :class="u.active ? 'btn-danger' : ''" type="button" :disabled="u.id === auth.user?.id" @click="update(u, { active: !u.active })">
                {{ u.active ? 'Suspend' : 'Restore' }}
              </button>
            </td>
            <td class="faint">{{ when(u.created_at) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-else id="panel-audit" role="tabpanel" class="pane">
      <div class="table-wrap">
        <table class="table">
          <thead><tr><th>When</th><th>Who</th><th>Action</th><th>Target</th><th>IP</th><th>Detail</th></tr></thead>
          <tbody>
            <tr v-for="r in audit?.items ?? []" :key="r.id">
              <td class="faint">{{ when(r.created_at) }}</td>
              <td>{{ r.actor_email ?? '—' }}</td>
              <td><code>{{ r.action }}</code></td>
              <td class="faint"><code v-if="r.target">{{ r.target.slice(0, 13) }}</code></td>
              <td class="faint">{{ r.ip }}</td>
              <td class="faint">{{ r.detail }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <PagerNav v-if="audit" :page="audit.page" :page-size="audit.page_size" :total="audit.total" @go="(p) => { page = p; loadAudit() }" />
    </div>
  </div>
</template>

<style scoped>
.pane {
  padding-top: 1.25rem;
}

.select {
  min-width: 10rem;
}
</style>
