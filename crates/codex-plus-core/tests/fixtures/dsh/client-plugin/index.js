// 宿主半边：给前端半边开一个 HTTP 路由（同 dsh 删除会话插件的写法），再注册一个斜杠命令。
export const name = 'client-demo'
export const inject = ['connection']

export function apply(ctx) {
  let hits = 0
  ctx.effect(() => ctx.connection.fetch.register({
    path: '/api/client-demo/echo',
    methods: ['POST'],
    requestBody: 'buffered',
    fetch: async (request) => {
      hits += 1
      const body = await request.json()
      return new Response(JSON.stringify({ ok: true, echo: body.text, hits }), { headers: { 'content-type': 'application/json' } })
    },
  }))
  ctx.get('commands')?.register({ name: 'client-demo', description: '前端演示', handler: () => ({ kind: 'success', text: `hits=${hits}` }) })
}
