// 模块解析钩子：插件自己装不到的 dsh 宿主包（peer 依赖）回落到 Codex++ 的替身实现。
// 插件包自带的真实依赖优先，只有解析失败时才用替身。

let shims = {}

export async function initialize(data) {
  shims = data?.shims ?? {}
}

export async function resolve(specifier, context, nextResolve) {
  try {
    return await nextResolve(specifier, context)
  } catch (error) {
    const shim = shims[specifier]
    if (shim && (error?.code === 'ERR_MODULE_NOT_FOUND' || error?.code === 'ERR_PACKAGE_PATH_NOT_EXPORTED')) {
      return { url: shim, format: 'module', shortCircuit: true }
    }
    throw error
  }
}
