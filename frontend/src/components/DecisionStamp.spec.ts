import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DecisionStamp from './DecisionStamp.vue'

describe('DecisionStamp', () => {
  it('shows the stamp wording and an accessible label', () => {
    const w = mount(DecisionStamp, { props: { decision: 'NOT_ADMISSIBLE', caption: 'Pay ₹0' } })
    expect(w.text()).toContain('Not admissible')
    expect(w.text()).toContain('Pay ₹0')
    expect(w.attributes('aria-label')).toBe('Decision: Not admissible')
    expect(w.classes()).not.toContain('press')
  })

  it('animates only when asked', () => {
    const w = mount(DecisionStamp, { props: { decision: 'NEEDS_REVIEW', press: true } })
    expect(w.classes()).toContain('press')
  })
})
