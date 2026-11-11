export interface LinkPreview {
  groupId: string
  title: string
  cwd?: string
  verdict: string
  previewToken?: string
  availableModes: string[]
  archiveAction?: string
  defaultChecked: boolean
  reason?: string
  extraB?: number
  recordCount?: { source: number; target: number; baseline?: number }
}
export const syncMode = (g: LinkPreview) =>
  g.previewToken
    ? g.availableModes[0] || (g.archiveAction === 'statusOnly' ? 'statusOnly' : null)
    : null
export const directSync = (g: LinkPreview) => syncMode(g) === 'fastForward'
export const syncSelections = (groups: LinkPreview[], checked: Set<string>) =>
  groups.flatMap((g) =>
    checked.has(g.groupId) && syncMode(g)
      ? [{ groupId: g.groupId, previewToken: g.previewToken, mode: syncMode(g) }]
      : []
  )
export const sessionKey = (s: { id: string; workspaceHash?: string }) =>
  `${s.workspaceHash || ''}:${s.id}`

export function canCopySession(
  preview: { supported?: boolean; sameAccount?: boolean; sourceUid?: string },
  session: { hasHistory?: boolean }
): boolean {
  return (
    preview.supported !== false &&
    !preview.sameAccount &&
    !!preview.sourceUid &&
    session.hasHistory === true
  )
}

// Switching can succeed even when individual session operations fail.
// Copy/sync use a boolean recovery flag; the recovery report carries issue arrays.
export function migrationIssues(
  result: any,
  recoveryNotice = '会话写入尚未完成，需要恢复；请重新检查后重试'
): string[] {
  return ['sessionCopy', 'sessionSync', 'sessionRecovery'].flatMap((key) => {
    const r = result[key]
    if (!r) return []
    return [
      ...(r.error ? [String(r.error)] : []),
      ...(r.needsRecovery === true ? [recoveryNotice] : []),
      ...['errors', 'skipped', 'linkErrors', 'needsRecovery', 'temporaryFiles'].flatMap((field) =>
        (Array.isArray(r[field]) ? r[field] : [])
          .filter((e: any) => e != null)
          .map((e: any) =>
            typeof e === 'string'
              ? e
              : [
                  e.title || e.id || e.conversationId || e.sessionId || e.groupId || e.operationId,
                  e.error || e.message || e.reason || JSON.stringify(e),
                ]
                  .filter(Boolean)
                  .join('：')
          )
      ),
    ]
  })
}
