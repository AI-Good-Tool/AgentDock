<script setup lang="ts">
import { IconExclamationCircleFill } from '@arco-design/web-vue/es/icon'
import { t } from '../i18n'

defineProps<{ expired: boolean; disabled: boolean }>()
</script>

<template>
  <div class="account-auth-alert" role="status">
    <IconExclamationCircleFill />
    <div>
      <strong>{{ expired ? t('登录已过期，请重新登录', 'Login expired — sign in again') : t('登录已失效，请重新登录', 'Login invalid — sign in again') }}</strong>
      <p>{{ disabled ? t('反代已停用；重新登录后请手动开启反代。', 'Proxy disabled. Sign in again, then enable proxy manually.') : t('登录凭证不可用；反代请求将跳过此账号。', 'Login unavailable. Proxy requests will skip this account.') }}</p>
    </div>
  </div>
</template>
