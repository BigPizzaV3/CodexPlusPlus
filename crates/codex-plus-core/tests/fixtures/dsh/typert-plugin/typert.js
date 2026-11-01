// 仿 dsh typert 生成器产出的宿主清单（TYPERT）：严格 codec 的 create() 给出解析器（这里手写一个最小的 zod 风格 parse）。
const text = { mode: 'strict', typeSymbol: 'string', create: () => ({ parse(value) { if (typeof value !== 'string') throw new Error('expected string'); return value } }) }
const json = { mode: 'src-json' }

export const TYPERT = {
  package: 'dsh-typert-demo',
  face: 'host',
  schemas: [{ name: 'NoteText', create: text.create }],
  model: { services: [], events: [], objects: [] },
  invocations: [
    { id: 'notes.add', service: 'notes', namespace: 'notes', method: 'add', invocation: { kind: 'direct' }, parameters: [{ name: 'text', wire: 'text', source: 'json', codec: text }], result: json },
    { id: 'notes.list', service: 'notes', namespace: 'notes', method: 'list', invocation: { kind: 'direct' }, parameters: [], result: json },
    { id: 'notes.read', service: 'notes', namespace: 'notes', method: 'read', invocation: { kind: 'direct' }, parameters: [{ name: 'note', wire: 'noteId', source: 'lookup', lookup: 'note', codec: json }], result: json },
    { id: 'notes.watch', service: 'notes', namespace: 'notes', method: 'watch', mode: 'stream', uplink: { codec: text }, invocation: { kind: 'direct' }, parameters: [{ name: 'prefix', wire: 'prefix', source: 'json', codec: json }], result: json },
  ],
}
