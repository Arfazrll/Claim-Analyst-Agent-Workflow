<script setup lang="ts">
import { computed, ref } from 'vue'
import { api, ApiError } from '@/api/client'
import type { ClaimSummary, Decision, Review } from '@/api/types'
import { useAuth } from '@/stores/auth'
import { useToasts } from '@/stores/toast'
import { DECISIONS, DECISION_LABEL, when } from '@/utils/format'
import DecisionTag from '@/components/DecisionTag.vue'

const props = defineProps<{ claim: ClaimSummary; reviews: Review[] }>()
const emit = defineEmits<{ done: [] }>()
const auth = useAuth()
const toasts = useToasts()

const action = ref<'agree' | 'override' | 'escalate'>('agree')
const finalDecision = ref<Decision>('NEEDS_REVIEW')
const notes = ref('')
const busy = ref(false)
const error = ref('')

const ownClaim = computed(() => props.claim.created_by === auth.user?.id && !auth.isAdmin)
const canReview = computed(() => auth.isReviewer && !ownClaim.value)
const alternatives = computed(() => DECISIONS.filter((d) => d !== props.claim.decision))

const ACTION_LABEL = { agree: 'Agreed with the engine', override: 'Overrode the decision', escalate: 'Escalated' } as const
const SUBMIT_LABEL = { agree: 'Sign off', override: 'Record override', escalate: 'Escalate claim' } as const

async function submit() {
  busy.value = true
  error.value = ''
  try {
    await api.post(`/api/claims/${props.claim.id}/review`, {
      action: action.value,
      final_decision: action.value === 'override' ? finalDecision.value : undefined,
      notes: notes.value,
    })
    toasts.push(action.value === 'agree' ? 'Signed off and written to the ledger.' : 'Review recorded and written to the ledger.', 'ok')
    notes.value = ''
    emit('done')
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'The review could not be saved.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="stack">
    <section v-if="reviews.length">
      <h3>Review history</h3>
      <ol class="history">
        <li v-for="r in reviews" :key="r.id">
          <div class="h-head">
            <strong>{{ ACTION_LABEL[r.action] }}</strong>
            <DecisionTag :decision="r.final_decision" />
            <span class="faint">{{ r.reviewer_name ?? 'Reviewer' }} · {{ when(r.created_at) }}</span>
          </div>
          <p v-if="r.notes" class="notes">{{ r.notes }}</p>
        </li>
      </ol>
    </section>

    <form v-if="canReview" class="form" @submit.prevent="submit">
      <h3>Your review</h3>
      <fieldset class="choices">
        <legend class="sr-only">Review action</legend>
        <label class="check"><input v-model="action" type="radio" value="agree" /> Agree with {{ DECISION_LABEL[claim.decision].toLowerCase() }}</label>
        <label class="check"><input v-model="action" type="radio" value="override" /> Override with another decision</label>
        <label class="check"><input v-model="action" type="radio" value="escalate" /> Escalate for a second opinion</label>
      </fieldset>
      <label v-if="action === 'override'" class="field">
        <span>Final decision</span>
        <select v-model="finalDecision" class="select">
          <option v-for="d in alternatives" :key="d" :value="d">{{ DECISION_LABEL[d] }}</option>
        </select>
      </label>
      <label class="field">
        <span>Notes {{ action === 'agree' ? '(optional)' : '(required)' }}</span>
        <textarea v-model="notes" class="textarea" maxlength="2000" :required="action !== 'agree'" :minlength="action === 'agree' ? 0 : 10" placeholder="What did you check, and why?"></textarea>
        <small>Notes are encrypted at rest. Only their fingerprint goes on the ledger.</small>
      </label>
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <div><button class="btn btn-primary" type="submit" :disabled="busy">{{ busy ? 'Saving…' : SUBMIT_LABEL[action] }}</button></div>
    </form>
    <p v-else-if="ownClaim" class="muted">You submitted this claim, so another reviewer has to sign it off.</p>
    <p v-else class="muted">A reviewer signs off or overrides the engine's decision. Your role can read reviews but not record them.</p>
  </div>
</template>

<style scoped>
.stack {
  display: grid;
  gap: 1.75rem;
}

.history {
  list-style: none;
  margin: 0.75rem 0 0;
  padding: 0;
  display: grid;
  gap: 0.75rem;
}

.history li {
  border-left: 2px solid var(--rule-strong);
  padding-left: 1rem;
}

.h-head {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 0.85rem;
  align-items: baseline;
  font-size: var(--t-sm);
}

.h-head .faint {
  font-size: var(--t-xs);
}

.notes {
  white-space: pre-line;
  font-size: var(--t-sm);
  margin-top: 0.35rem;
}

.form {
  display: grid;
  gap: 1rem;
  max-width: 40rem;
}

.choices {
  border: 0;
  padding: 0;
  margin: 0;
  display: grid;
  gap: 0.5rem;
}
</style>
