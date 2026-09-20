# 用户脚本热重载

脚本市场中「刷新本地」右侧的「热重载脚本」读取当前文件及启停开关，无需重启 Codex++ 进程。按钮执行期间禁用，避免重复点击。未连接 Codex 时显示错误，不修改脚本开关。

支持清理接口的脚本在原页面内重载；未提供清理接口、清理抛错或返回 Promise 的脚本回退到刷新 Codex 页面，由浏览器释放旧计时器、监听器、观察器和 DOM。页面临时 UI 状态可能随刷新重置。旧版 launcher 需要先重启升级后的 Codex++ 才能启用这种安全回退。

脚本可以在同步初始化期间注册一个或多个同步清理函数：

```js
const timer = setInterval(update, 1000);
window.__codexPlusUserScripts?.registerCleanup?.(() => {
  clearInterval(timer);
  observer.disconnect();
  window.removeEventListener("message", onMessage);
  root.remove();
});
```

清理应取消未完成的工作，或确保异步回调在销毁后不再修改页面/存储；注册接口并不会自动代管脚本资源。清理按脚本和注册顺序逆序执行，全部关闭或删除也会触发清理。

页面启动只发起一次脚本加载请求，从磁盘读取当前配置，不缓存启动时的脚本包。不增加后台轮询或文件监听器；现有桥接负责加载，管理页通过短连接触发重载。

自动加载使用独立的 `/user-scripts/load` 入口，同一页面上的重复请求或桥接恢复不会重新执行已加载脚本，也不会触发页面刷新。只有显式的 `/user-scripts/reload` 才执行清理或刷新回退，避免旧脚本在页面恢复后再次触发刷新循环。更新脚本文件或开关后，点击「热重载脚本」应用变更。
