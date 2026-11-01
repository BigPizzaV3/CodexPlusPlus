// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-fs 替身：错误类。文件系统本身是宿主注入的 ctx.fs（见 services.mjs）。

export class FsError extends Error {
  constructor(message, code = 'FS_IO_ERROR', options) {
    super(message, options)
    this.name = 'FsError'
    this.code = code
  }
}
