export async function listen<T = unknown>(_event: string, _handler: (event: T) => void): Promise<() => void> {
  return () => {};
}
