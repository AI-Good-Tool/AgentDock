import { createApp } from 'vue'
import ArcoVue from '@arco-design/web-vue'
import '@arco-design/web-vue/dist/arco.css'
import App from './App.vue'
import './theme-tokens.css'
import './wb-style.css'
import './components/workbuddy/AccountCard.css'
import './app.css'

createApp(App).use(ArcoVue).mount('#app')
