// 由 Codex++ 生成配置；只使用 Claude Code 官方 Mods API。
const CONFIG = /*CODEXPP_CONFIG*/{};
const EXTENSIONS = CONFIG.extensions || [];
const PLUGIN_NAME = 'codexpp-extension-host';
const TOOLS = EXTENSIONS.flatMap((extension, index) =>
  extension.tools.map(tool => ({
    extension,
    tool,
    name: 'cpx_' + index + '_' + tool.name,
  })),
);

function errorText(error) {
  return String(error?.message || error).slice(0, 300);
}

function resultText(value) {
  if (typeof value === 'string') return value;
  if (typeof value?.text === 'string') return value.text;
  return JSON.stringify(value ?? null);
}

function decodeResult(response) {
  if (response?.isError) {
    throw new Error(response.content?.find(item => item.type === 'text')?.text || 'Extension backend failed');
  }
  if (response?.structuredContent && Object.hasOwn(response.structuredContent, 'result')) {
    return response.structuredContent.result;
  }
  const text = response?.content?.find(item => item.type === 'text')?.text;
  if (typeof text !== 'string') throw new Error('Extension backend returned no result');
  return JSON.parse(text);
}

async function callBackend($, extension, method, params, sessionId) {
  const connection = await $.mcp.connect('codexpp-extensions');
  if (!connection.isConnected) {
    throw new Error(connection.message || connection.reason || 'Extension backend is unavailable');
  }
  const response = await $.mcp.call(connection.server || CONFIG.serverName, CONFIG.toolName, {
    key: extension.key,
    method,
    params,
    sessionId,
  });
  return decodeResult(response);
}

async function reportFailure($, extension, error) {
  try {
    await $.ui.log(extension.name + ': ' + errorText(error));
  } catch {
    // 宿主退出时日志也可能不可用，不让观测事件阻断 Claude。
  }
}

async function forwardEvent($, event, e) {
  let sessionId;
  try {
    sessionId = await $.session.id();
  } catch {
    return;
  }
  const params = { host: 'claudeDesktop', event, sessionId };
  if (typeof e.turnId === 'string') params.turnId = e.turnId;
  if (typeof e.agentId === 'string') params.agentId = e.agentId;
  if (typeof e.isAborted === 'boolean') params.isAborted = e.isAborted;
  if (typeof e.reason === 'string') params.reason = e.reason;
  if (typeof e.source === 'string') params.source = e.source;
  for (const extension of EXTENSIONS) {
    const method = extension.events[event];
    if (!method) continue;
    try {
      await callBackend($, extension, method, params, sessionId);
    } catch (error) {
      await reportFailure($, extension, error);
    }
  }
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    for (const entry of TOOLS) {
      try {
        await $.tool.register({
          name: entry.name,
          description: entry.extension.name + ': ' + entry.tool.description,
          inputSchema: entry.tool.inputSchema,
        });
      } catch (error) {
        await reportFailure($, entry.extension, error);
      }
    }
    await forwardEvent($, 'session.start', e);
    return next(e);
  });

  on('classic.SessionStart', { source: ['clear', 'resume', 'fork'] }, async ($, e, next) => {
    await forwardEvent($, 'session.start', e);
    return next(e);
  });

  on('turn.start', async ($, e, next) => {
    await forwardEvent($, 'turn.start', e);
    return next(e);
  });

  on('turn.complete', async ($, e, next) => {
    await forwardEvent($, 'turn.complete', e);
    return next(e);
  });

  on('session.end', async ($, e, next) => {
    await forwardEvent($, 'session.end', e);
    return next(e);
  });

  on('tool.call', async ($, e, next) => {
    const entry = TOOLS.find(item => e.tool === 'mcp__' + PLUGIN_NAME + '__' + item.name);
    if (!entry) return next(e);
    try {
      const params = {};
      for (const key of Object.keys(entry.tool.inputSchema.properties || {})) {
        if (Object.hasOwn(e, key)) params[key] = e[key];
      }
      const sessionId = await $.session.id();
      const value = await callBackend($, entry.extension, entry.tool.method, params, sessionId);
      return { result: resultText(value) };
    } catch (error) {
      return { result: 'Extension failed: ' + errorText(error) };
    }
  });

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const theirs = await next(e);
    if (e.surface !== 'desktop') return theirs;
    try {
      const { Box, Button } = $.ui.resolve(e);
      const rows = EXTENSIONS.filter(extension => extension.buttons.length > 0).map(extension =>
        Box({
          key: extension.key,
          flexDirection: 'row',
          flexWrap: 'wrap',
          columnGap: 1,
          children: extension.buttons.map(button =>
            Button({
              key: extension.key + ':' + button.id,
              label: button.label,
              onPress: async () => {
                try {
                  const sessionId = await $.session.id();
                  const value = await callBackend($, extension, button.method, {
                    host: 'claudeDesktop',
                    source: 'button',
                    buttonId: button.id,
                    sessionId,
                  }, sessionId);
                  await $.ui.toast(resultText(value).slice(0, 500));
                } catch (error) {
                  await reportFailure($, extension, error);
                  try {
                    await $.ui.toast(extension.name + ': ' + errorText(error));
                  } catch {
                    // 插件卸载或窗口关闭后不再显示提示。
                  }
                }
              },
            }),
          ),
        }),
      );
      if (rows.length === 0) return theirs;
      return Box({ flexDirection: 'column', children: [theirs, ...rows] });
    } catch {
      return theirs;
    }
  });
}
