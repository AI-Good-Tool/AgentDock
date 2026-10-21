// Independent controls must not silently discard each other's operations.
export async function runKeyed(pending: Set<string>, id: string, work: () => Promise<void>) {
  if (pending.has(id)) return false
  pending.add(id)
  try {
    await work()
    return true
  } finally {
    pending.delete(id)
  }
}

// A slow polling response must not undo a newer mutation's refreshed snapshot.
export function latestRequest<T>(apply: (value: T) => void) {
  let generation = 0
  return async (load: () => Promise<T>) => {
    const current = ++generation
    const value = await load()
    if (current === generation) apply(value)
  }
}
