export default function create({ host }) {
  const events = [];
  let clicks = 0;
  let pageCalls = 0;
  return {
    recordEvent(event) {
      events.push(event);
      if (events.length > 20) events.shift();
      return { received: event.event };
    },
    // 页面里的按钮、Code 会话里的按钮和模型工具调的都是这一个方法、同一个后台实例。
    status(params = {}) {
      if (params?.source === 'button') clicks++;
      if (params?.source === 'page') pageCalls++;
      const latest = events.at(-1);
      return {
        text: `Claude 拓展已连接；Code 会话按钮 ${clicks} 次；页面调用 ${pageCalls} 次；事件 ${events.length} 条；最近：${latest?.event || '无'}`,
        host,
        clicks,
        pageCalls,
        events: events.slice(),
      };
    },
  };
}
