import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'
import type { Role } from '@/api/types'
import { useAuth } from '@/stores/auth'

declare module 'vue-router' {
  interface RouteMeta {
    public?: boolean
    role?: Role
    title?: string
  }
}

const routes: RouteRecordRaw[] = [
  { path: '/login', name: 'login', component: () => import('@/views/LoginView.vue'), meta: { public: true, title: 'Sign in' } },
  { path: '/register', name: 'register', component: () => import('@/views/RegisterView.vue'), meta: { public: true, title: 'Create account' } },
  {
    path: '/',
    component: () => import('@/layouts/AppShell.vue'),
    children: [
      { path: '', name: 'desk', component: () => import('@/views/DeskView.vue'), meta: { title: 'Desk' } },
      { path: 'claims', name: 'claims', component: () => import('@/views/ClaimsView.vue'), meta: { title: 'Claims' } },
      { path: 'claims/new', name: 'new-claim', component: () => import('@/views/NewClaimView.vue'), meta: { title: 'New claim' } },
      { path: 'claims/:id', name: 'claim', component: () => import('@/views/ClaimDetailView.vue'), props: true, meta: { title: 'Claim' } },
      { path: 'policy', name: 'policy', component: () => import('@/views/PolicyView.vue'), meta: { title: 'Policy' } },
      { path: 'ledger', name: 'ledger', component: () => import('@/views/LedgerView.vue'), meta: { title: 'Ledger', role: 'reviewer' } },
      { path: 'evaluation', name: 'evaluation', component: () => import('@/views/EvaluationView.vue'), meta: { title: 'Evaluation', role: 'reviewer' } },
      { path: 'admin', name: 'admin', component: () => import('@/views/AdminView.vue'), meta: { title: 'Administration', role: 'admin' } },
      { path: 'account', name: 'account', component: () => import('@/views/AccountView.vue'), meta: { title: 'Account' } },
    ],
  },
  { path: '/:pathMatch(.*)*', name: 'not-found', component: () => import('@/views/NotFoundView.vue'), meta: { public: true, title: 'Not found' } },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
  scrollBehavior: () => ({ top: 0 }),
})

router.beforeEach(async (to) => {
  const auth = useAuth()
  if (!auth.loaded) await auth.load()
  if (!to.meta.public && !auth.user) return { name: 'login', query: { next: to.fullPath } }
  if ((to.name === 'login' || to.name === 'register') && auth.user) return { name: 'desk' }
  if (to.meta.role && !auth.can(to.meta.role)) return { name: 'desk' }
  return true
})

router.afterEach((to) => {
  document.title = to.meta.title ? `${to.meta.title} · Claim Desk` : 'Claim Desk'
})
