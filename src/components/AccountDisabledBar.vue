<script setup lang="ts">
import { IconStop } from '@arco-design/web-vue/es/icon'
import type { ProviderKey } from '../api'
import { t } from '../i18n'
import { busy, setAccountDisabled } from '../store'

const props = defineProps<{ provider: ProviderKey; id: string }>()
</script>

<template>
  <div class="account-disabled-bar" role="status">
    <IconStop />
    <div>
      <strong>{{ t('账号已禁用', 'Account disabled') }}</strong>
      <p>{{ t('反代已关闭；不参与一键刷新积分 / 限额、全部签到与定时签到。', 'Proxy off; left out of batch credit / limit refreshes, check-in for all and scheduled check-ins.') }}</p>
    </div>
    <a-button size="mini" :loading="busy(`disable:${props.provider}/${props.id}`)" @click="setAccountDisabled(props.provider, props.id, false)">{{
      t('启用', 'Enable')
    }}</a-button>
  </div>
</template>
