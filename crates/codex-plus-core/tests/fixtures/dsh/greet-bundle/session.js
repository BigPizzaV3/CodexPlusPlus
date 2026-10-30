export const name = 'session-tool'
export const inject = ['tools', 'sessions']
export function apply() {
  throw new Error('should not be applied')
}
