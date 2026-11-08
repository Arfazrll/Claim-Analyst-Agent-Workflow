import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api, request } from '@/api/client'
import type { Role, User } from '@/api/types'

const RANK: Record<Role, number> = { analyst: 0, reviewer: 1, admin: 2 }

export const useAuth = defineStore('auth', () => {
  const user = ref<User | null>(null)
  const loaded = ref(false)

  const isReviewer = computed(() => !!user.value && RANK[user.value.role] >= RANK.reviewer)
  const isAdmin = computed(() => user.value?.role === 'admin')

  function can(role: Role) {
    return !!user.value && RANK[user.value.role] >= RANK[role]
  }

  /** Restore the session from the HttpOnly cookie (refreshing once if the access token expired). */
  async function load() {
    try {
      user.value = (await api.get<{ user: User }>('/api/auth/me')).user
    } catch {
      try {
        await request('/api/auth/refresh', { method: 'POST', noRefresh: true })
        user.value = (await api.get<{ user: User }>('/api/auth/me')).user
      } catch {
        user.value = null
      }
    } finally {
      loaded.value = true
    }
  }

  async function login(email: string, password: string) {
    user.value = (await request<{ user: User }>('/api/auth/login', { method: 'POST', body: { email, password }, noRefresh: true })).user
  }

  async function register(name: string, email: string, password: string) {
    user.value = (await request<{ user: User }>('/api/auth/register', { method: 'POST', body: { name, email, password }, noRefresh: true })).user
  }

  async function logout() {
    try {
      await request('/api/auth/logout', { method: 'POST', noRefresh: true })
    } finally {
      user.value = null
    }
  }

  return { user, loaded, isReviewer, isAdmin, can, load, login, register, logout }
})
