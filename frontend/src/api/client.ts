import type { FieldError } from './types'

/** A failed API call, with the server's message (never a stack trace). */
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public fieldErrors: FieldError[] = [],
    public retryAfter?: number,
  ) {
    super(message)
  }
}

export function readCookie(name: string): string | null {
  const prefix = `${name}=`
  for (const part of document.cookie.split(';')) {
    const p = part.trim()
    if (p.startsWith(prefix)) return decodeURIComponent(p.slice(prefix.length))
  }
  return null
}

const UNSAFE = new Set(['POST', 'PUT', 'PATCH', 'DELETE'])
let refreshing: Promise<boolean> | null = null
let onSessionExpired: (() => void) | null = null

export function setSessionExpiredHandler(fn: () => void) {
  onSessionExpired = fn
}

/** One refresh at a time: concurrent 401s wait for the same rotation. */
function refreshSession(): Promise<boolean> {
  refreshing ??= fetch('/api/auth/refresh', { method: 'POST', credentials: 'same-origin' })
    .then((r) => r.ok)
    .catch(() => false)
    .finally(() => {
      refreshing = null
    })
  return refreshing
}

export interface RequestOptions {
  method?: string
  body?: unknown
  headers?: Record<string, string>
  signal?: AbortSignal
  /** Skip the automatic refresh-and-retry on 401 (used by auth endpoints). */
  noRefresh?: boolean
}

export async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
  const method = (opts.method ?? 'GET').toUpperCase()
  const send = () => {
    const headers: Record<string, string> = { Accept: 'application/json', ...opts.headers }
    if (opts.body !== undefined) headers['Content-Type'] = 'application/json'
    if (UNSAFE.has(method)) {
      const csrf = readCookie('caw_csrf')
      if (csrf) headers['X-CSRF-Token'] = csrf
    }
    return fetch(path, {
      method,
      headers,
      body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
      credentials: 'same-origin',
      signal: opts.signal,
    })
  }

  let res = await send()
  if (res.status === 401 && !opts.noRefresh && !path.startsWith('/api/auth/')) {
    if (await refreshSession()) {
      res = await send()
    } else {
      onSessionExpired?.()
    }
  }

  if (res.status === 204) return undefined as T
  const text = await res.text()
  let data: unknown = null
  try {
    data = text ? JSON.parse(text) : null
  } catch {
    data = null
  }
  if (!res.ok) {
    const d = (data ?? {}) as { error?: string; message?: string; errors?: FieldError[]; retry_after_secs?: number }
    throw new ApiError(res.status, d.error ?? 'error', d.message ?? `Request failed (${res.status}).`, d.errors ?? [], d.retry_after_secs)
  }
  return data as T
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>(path, { signal }),
  post: <T>(path: string, body?: unknown, headers?: Record<string, string>) => request<T>(path, { method: 'POST', body: body ?? {}, headers }),
  patch: <T>(path: string, body: unknown) => request<T>(path, { method: 'PATCH', body }),
}

/** Download a JSON response as a file (authenticated, same origin). */
export async function downloadJson(path: string, filename: string) {
  const data = await request<unknown>(path)
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}
