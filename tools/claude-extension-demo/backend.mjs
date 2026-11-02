export default function create({ host }) {
  const events = [];
  let clicks = 0;
  return {
    recordEvent(event) {
      events.push(event);
      if (events.length > 20) events.shift();
      return { received: event.event };
    },
    status(params = {}) {
      if (params.source === 'button') clicks++;
      const latest = events.at(-1);
      return {
        text: `Claude 拓展已连接；按钮 ${clicks} 次；事件 ${events.length} 条；最近：${latest?.event || '无'}`,
        host,
        clicks,
        events: events.slice(),
      };
    },
  };
}
