<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api/client'
import type { LedgerEntry, Paged } from '@/api/types'
import PagerNav from '@/components/PagerNav.vue'
import { useAuth } from '@/stores/auth'
import { useToasts } from '@/stores/toast'
import { shortHash, when } from '@/utils/format'

interface LedgerStatus {
  head: { seq: number; hash: string }
  entries_by_status: [string, number][]
  mode: 'blockchain' | 'ipfs' | 'local-hash-chain'
  chain: { enabled: boolean; contract?: string; chain_id?: number; signer?: string; reachable?: boolean; block_number?: number; signer_is_writer?: boolean; error?: string }
  ipfs: { enabled: boolean; reachable?: boolean; gateway?: string }
}
interface ChainCheck {
  ok: boolean
  checked: number
  first_broken_seq: number | null
  detail: string
}

const auth = useAuth()
const toasts = useToasts()
const status = ref<LedgerStatus | null>(null)
const entries = ref<Paged<LedgerEntry> | null>(null)
const check = ref<ChainCheck | null>(null)
const verifying = ref(false)
const page = ref(1)

const MODE = { blockchain: 'Anchored to an EVM chain', ipfs: 'Pinned to IPFS', 'local-hash-chain': 'Local hash chain only' } as const

async function loadEntries() {
  entries.value = await api.get<Paged<LedgerEntry>>(`/api/ledger/entries?page=${page.value}&page_size=25`)
}

onMounted(async () => {
  status.value = await api.get<LedgerStatus>('/api/ledger/status')
  await loadEntries()
})

async function verify() {
  verifying.value = true
  try {
    check.value = await api.get<ChainCheck>('/api/ledger/verify')
  } finally {
    verifying.value = false
  }
}

async function requeue() {
  const r = await api.post<{ requeued: number }>('/api/ledger/requeue')
  toasts.push(`${r.requeued} failed entr${r.requeued === 1 ? 'y' : 'ies'} queued again.`, 'ok')
  await loadEntries()
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Ledger</h1>
        <p>Every claim event, hash-chained in order. Changing any past record breaks every hash after it.</p>
      </div>
      <div class="actions">
        <button v-if="auth.isAdmin" class="btn" type="button" @click="requeue">Retry failed anchors</button>
        <button class="btn btn-primary" type="button" :disabled="verifying" @click="verify">{{ verifying ? 'Checking…' : 'Verify whole chain' }}</button>
      </div>
    </header>

    <section v-if="check" class="result" :data-ok="check.ok">
      <h2>{{ check.ok ? 'Chain intact' : `Chain broken at entry #${check.first_broken_seq}` }}</h2>
      <p class="muted">{{ check.detail }}</p>
    </section>

    <section v-if="status" class="status">
      <dl>
        <div><dt>Mode</dt><dd>{{ MODE[status.mode] }}</dd></div>
        <div><dt>Head</dt><dd>#{{ status.head.seq }} <code :title="status.head.hash">{{ shortHash(status.head.hash, 10) }}</code></dd></div>
        <div v-if="status.chain.enabled">
          <dt>Contract</dt>
          <dd><code>{{ shortHash(status.chain.contract, 8) }}</code> on chain {{ status.chain.chain_id }}, <span :class="status.chain.reachable ? '' : 'error-text'">{{ status.chain.reachable ? `block ${status.chain.block_number}` : 'unreachable' }}</span></dd>
        </div>
        <div v-if="status.chain.enabled"><dt>Signer</dt><dd><code>{{ shortHash(status.chain.signer, 8) }}</code> {{ status.chain.signer_is_writer ? 'is an authorised writer' : 'is NOT an authorised writer' }}</dd></div>
        <div v-if="status.ipfs.enabled"><dt>IPFS</dt><dd>{{ status.ipfs.reachable ? 'reachable' : 'unreachable' }}</dd></div>
        <div><dt>Entries</dt><dd>{{ status.entries_by_status.map(([s, n]) => `${n} ${s}`).join(', ') || 'none yet' }}</dd></div>
      </dl>
    </section>

    <div class="table-wrap">
      <table class="table">
        <thead><tr><th>#</th><th>Event</th><th>Claim</th><th>Entry hash</th><th>Status</th><th>Transaction</th><th>Recorded</th></tr></thead>
        <tbody>
          <tr v-for="e in entries?.items ?? []" :key="e.seq">
            <td>{{ e.seq }}</td>
            <td>{{ e.kind.replace('claim_', '') }}</td>
            <td><RouterLink :to="`/claims/${e.claim_id}`">open</RouterLink></td>
            <td><code :title="e.entry_hash">{{ shortHash(e.entry_hash, 10) }}</code></td>
            <td :class="{ 'error-text': e.chain_status === 'failed' }" :title="e.last_error ?? ''">{{ e.chain_status }}</td>
            <td><code v-if="e.tx_hash" :title="e.tx_hash">{{ shortHash(e.tx_hash, 8) }}</code><span v-else class="faint">—</span></td>
            <td class="faint">{{ when(e.created_at) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <PagerNav v-if="entries" :page="entries.page" :page-size="entries.page_size" :total="entries.total" @go="(p) => { page = p; loadEntries() }" />
  </div>
</template>

<style scoped>
.actions {
  display: flex;
  gap: 0.5rem;
}

.result {
  border-left: 3px solid var(--ok);
  padding: 0.25rem 0 0.25rem 1rem;
  margin-bottom: 1.5rem;
}

.result[data-ok='false'] {
  border-left-color: var(--bad);
}

.status dl {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
  gap: 1rem 2rem;
  margin: 0 0 2rem;
  padding: 1.25rem;
  background: var(--surface);
  border: 1px solid var(--rule);
  border-radius: var(--r-md);
}

.status dt {
  font-size: var(--t-xs);
  color: var(--ink-faint);
}

.status dd {
  margin: 0;
  font-size: var(--t-sm);
}
</style>
