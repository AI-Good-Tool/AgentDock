// Only visible, still-existing accounts can participate in a batch action.
export function visibleSelection(selected: string[], visible: string[]): string[] {
  const scope = new Set(visible)
  return [...new Set(selected)].filter((id) => scope.has(id))
}
export function toggleSelection(selected: string[], id: string, checked: boolean): string[] {
  return checked ? [...new Set([...selected, id])] : selected.filter((value) => value !== id)
}
export const accountKey = (provider: string, id: string) => JSON.stringify([provider, id])
