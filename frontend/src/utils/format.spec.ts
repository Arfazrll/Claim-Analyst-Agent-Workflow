import { describe, expect, it } from 'vitest'
import { humanize, money, shortHash } from './format'

describe('format', () => {
  it('formats rupees with Indian grouping', () => {
    expect(money(190500)).toBe('₹1,90,500')
    expect(money(null)).toBe('—')
    expect(money(0)).toBe('₹0')
  })

  it('shortens hashes', () => {
    expect(shortHash('a'.repeat(64), 6)).toBe('aaaaaa…aaaa')
    expect(shortHash(null)).toBe('—')
  })

  it('humanizes dimension keys', () => {
    expect(humanize('pre_post_hospitalization_window')).toBe('Pre post hospitalization window')
  })
})
