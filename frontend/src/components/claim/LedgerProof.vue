<script setup lang="ts">
import { computed, ref } from 'vue'
import { api } from '@/api/client'
import type { LedgerEntry, Proof } from '@/api/types'
import { shortHash, when } from '@/utils/format'

const props = defineProps<{ claimId: string; entries: LedgerEntry[] }>()
const proof = ref<Proof | null>(null)
const busy = ref(false)
const error = ref('')
// After a verification, show the live anchoring status returned with the proof.
const shown = computed(() => proof.value?.entries ?? props.entries)

const STATUS: Record<LedgerEntry['chain_status'], string> = {
  local: 'Hash-chained locally',
  pending: 'Waiting to anchor',
  retry: 'Retrying anchor',
  pinned: 'Pinned to IPFS',
  anchored: 'Anchored on-chain',
  failed: 'Anchoring failed',
}

async function verify() {
  busy.value = true
  error.value = ''
  try {
    proof.value = await api.get<Proof>(`/api/claims/${props.claimId}/proof`)
  } catch (e) {
    error.value = (e as Error).message
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="stack">
    <p class="muted lead">
      Each event on this claim is hash-chained to every earlier event. The record itself stays encrypted; only its fingerprint is pinned to IPFS and anchored to the
      ClaimLedger contract, so anyone can check it was not altered.
    </p>

    <ol class="entries">
      <li v-for="e in shown" :key="e.seq">
        <div class="e-head">
          <h3>#{{ e.seq }} {{ e.kind.replace('claim_', '').replace('_', ' ') }}</h3>
          <span class="status" :data-status="e.chain_status">{{ STATUS[e.chain_status] }}</span>
          <span class="faint">{{ when(e.created_at) }}</span>
        </div>
        <dl>
          <dt>Entry hash</dt><dd><code :title="e.entry_hash">{{ shortHash(e.entry_hash, 16) }}</code></dd>
          <dt>Previous</dt><dd><code :title="e.prev_hash">{{ shortHash(e.prev_hash, 16) }}</code></dd>
          <template v-if="e.tx_hash">
            <dt>Transaction</dt>
            <dd>
              <a v-if="e.tx_url" :href="e.tx_url" target="_blank" rel="noopener noreferrer"><code>{{ shortHash(e.tx_hash, 12) }}</code></a>
              <code v-else :title="e.tx_hash">{{ shortHash(e.tx_hash, 12) }}</code>
              <span class="faint"> block {{ e.block_number }}</span>
            </dd>
          </template>
          <template v-if="e.cid">
            <dt>IPFS</dt>
            <dd>
              <a v-if="e.ipfs_url" :href="e.ipfs_url" target="_blank" rel="noopener noreferrer"><code>{{ shortHash(e.cid, 12) }}</code></a>
              <code v-else>{{ shortHash(e.cid, 12) }}</code>
            </dd>
          </template>
          <template v-if="e.last_error">
            <dt>Last error</dt><dd class="error-text">{{ e.last_error }}</dd>
          </template>
        </dl>
      </li>
    </ol>

    <div>
      <button class="btn btn-primary" type="button" :disabled="busy" @click="verify">{{ busy ? 'Verifying…' : 'Verify integrity' }}</button>
      <p v-if="error" class="error-text">{{ error }}</p>
    </div>

    <section v-if="proof" class="proof" :data-ok="proof.ok">
      <h3>{{ proof.ok ? 'Integrity verified' : 'Integrity check failed' }}</h3>
      <p class="faint small">
        Record <code>{{ shortHash(proof.record_id, 12) }}</code>
        <template v-if="proof.chain"> · contract <code>{{ shortHash(proof.chain.contract, 8) }}</code> on chain {{ proof.chain.chain_id }}</template>
        · {{ when(proof.verified_at) }}
      </p>
      <ul>
        <li v-for="(c, i) in proof.checks" :key="i" :class="{ fail: !c.passed }">
          {{ c.passed ? 'Passed' : 'Failed' }}: {{ c.name }}<span v-if="c.detail" class="faint"> — {{ c.detail }}</span>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.stack {
  display: grid;
  gap: 1.25rem;
}

.lead,
.small {
  font-size: var(--t-sm);
}

.entries {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 0.75rem;
}

.entries > li {
  padding: 0.9rem 1rem;
  border: 1px solid var(--rule);
  border-radius: var(--r-md);
  background: var(--surface);
}

.e-head {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 0.9rem;
  align-items: baseline;
}

.e-head h3 {
  text-transform: capitalize;
}

.e-head .faint {
  margin-left: auto;
  font-size: var(--t-xs);
}

.status {
  font-size: var(--t-xs);
  font-weight: 620;
  color: var(--warn);
}

.status[data-status='anchored'],
.status[data-status='pinned'],
.status[data-status='local'] {
  color: var(--ok);
}

.status[data-status='failed'] {
  color: var(--bad);
}

dl {
  display: grid;
  grid-template-columns: 7rem 1fr;
  gap: 0.25rem 1rem;
  margin: 0.6rem 0 0;
  font-size: var(--t-sm);
}

dt {
  color: var(--ink-faint);
}

dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
}

.proof {
  border-left: 3px solid var(--ok);
  padding-left: 1rem;
}

.proof[data-ok='false'] {
  border-left-color: var(--bad);
}

.proof ul {
  margin: 0.5rem 0 0;
  padding-left: 1.1rem;
  font-size: var(--t-sm);
  display: grid;
  gap: 0.2rem;
}

.proof .fail {
  color: var(--bad);
}
</style>
