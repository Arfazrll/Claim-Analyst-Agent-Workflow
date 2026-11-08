import '@fontsource-variable/archivo/wdth.css'
import '@fontsource/ibm-plex-mono/400.css'
import './styles/main.css'

import { createPinia } from 'pinia'
import { createApp } from 'vue'
import App from './App.vue'
import { setSessionExpiredHandler } from './api/client'
import { router } from './router'
import { useAuth } from './stores/auth'

const app = createApp(App)
app.use(createPinia())
app.use(router)

setSessionExpiredHandler(() => {
  useAuth().user = null
  if (router.currentRoute.value.meta.public !== true) {
    router.push({ name: 'login', query: { next: router.currentRoute.value.fullPath, expired: '1' } })
  }
})

try {
  const saved = localStorage.getItem('theme')
  if (saved === 'light' || saved === 'dark') document.documentElement.dataset.theme = saved
} catch {
  /* storage unavailable: follow the system theme */
}

app.mount('#app')
