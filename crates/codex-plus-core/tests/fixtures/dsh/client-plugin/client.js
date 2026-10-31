// 前端半边：懒 CJS 工厂，React 由宿主提供；往全局浮层与会话「…」菜单各放一个组件。
window.__ModuleLoader__.load({
  id: 'dsh-client-demo',
  factory(require) {
    const React = require('react')
    const h = React.createElement
    function Badge({ t }) {
      const [text, setText] = React.useState('…')
      React.useEffect(() => {
        fetch('/api/client-demo/echo', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: 'ping' }) })
          .then((response) => response.json())
          .then((value) => setText(`${t('badge')}:${value.echo}:${value.hits}`))
      }, [])
      return h('div', { 'data-dsh-client-demo': 'badge' }, text)
    }
    function MenuItem({ sessionId, displayTitle, useMenuOpenState }) {
      const [, setOpen] = useMenuOpenState()
      return h('button', { type: 'button', 'data-dsh-client-demo': 'menu', onClick: () => setOpen(false) }, `demo ${sessionId}:${displayTitle}`)
    }
    return {
      inject: ['slots', 'locale'],
      apply(ctx) {
        ctx.locale.register('clientDemo', { zh: { badge: '徽标' }, en: { badge: 'badge' } })
        ctx.slots.inject('shell.overlay', () => ctx.slots.register({ name: 'shell.overlay', id: 'demo-badge', locale: 'clientDemo' }, Badge))
        ctx.slots.inject('sidebar.workspaces.session.menu.item', () => ctx.slots.register({ name: 'sidebar.workspaces.session.menu.item', id: 'demo-menu', order: 500 }, MenuItem))
        ctx.slots.inject('conversation.hero.brand.mark', () => ctx.slots.register({ name: 'conversation.hero.brand.mark' }, () => null))
      },
    }
  },
})
