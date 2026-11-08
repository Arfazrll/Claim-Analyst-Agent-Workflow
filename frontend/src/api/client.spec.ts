import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, readCookie, request } from './client'

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

afterEach(() => {
  vi.restoreAllMocks()
  document.cookie = 'caw_csrf=; max-age=0'
})

describe('api client', () => {
  it('reads cookies', () => {
    document.cookie = 'caw_csrf=tok%20en'
    expect(readCookie('caw_csrf')).toBe('tok en')
    expect(readCookie('missing')).toBeNull()
  })

  it('sends the CSRF token on unsafe requests only', async () => {
    document.cookie = 'caw_csrf=abc'
    const f = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => json(200, { ok: true }))
    await request('/api/x', { method: 'POST', body: {} })
    await request('/api/x')
    const post = f.mock.calls[0]![1]!.headers as Record<string, string>
    const get = f.mock.calls[1]![1]!.headers as Record<string, string>
    expect(post['X-CSRF-Token']).toBe('abc')
    expect(get['X-CSRF-Token']).toBeUndefined()
  })

  it('refreshes once on 401 and retries', async () => {
    const f = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(json(401, { error: 'unauthorized' }))
      .mockResolvedValueOnce(json(200, {}))
      .mockResolvedValueOnce(json(200, { value: 42 }))
    const r = await request<{ value: number }>('/api/claims')
    expect(r.value).toBe(42)
    expect(f.mock.calls[1]![0]).toBe('/api/auth/refresh')
  })

  it('surfaces server messages and field errors', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(422, { error: 'validation_error', message: 'Bad', errors: [{ field: 'case_id', message: 'required' }] }))
    await expect(request('/api/claims', { method: 'POST', body: {}, noRefresh: true })).rejects.toMatchObject({
      status: 422,
      message: 'Bad',
      fieldErrors: [{ field: 'case_id', message: 'required' }],
    })
    expect(new ApiError(400, 'x', 'y')).toBeInstanceOf(Error)
  })
})
