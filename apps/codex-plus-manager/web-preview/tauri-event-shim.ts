export async function listen<T = unknown>(_event: string, _handler: (event: T) => void): Promise<() => void> {
  return () => {};
}

export async function emit(_event: string, _payload?: unknown): Promise<void> {}
