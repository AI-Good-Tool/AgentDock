import { t } from '../i18n'
export function apiKeyIssue(value: string): string {
  return /^[\x21-\x7e]{16,256}$/.test(value)
    ? ''
    : t('请输入 16–256 个非空白 ASCII 字符，不能包含空格或换行', 'Use 16–256 visible ASCII characters, no spaces or line breaks')
}
