'use strict';

const path = require('path');
const Module = require('module');

const engineRoot = process.env.XUANPLUS_HVIGOR_ENGINE;
const pluginRoot = process.env.XUANPLUS_HVIGOR_PLUGIN;

if (engineRoot && pluginRoot) {
  const originalResolveFilename = Module._resolveFilename;
  Module._resolveFilename = function (request, parent, isMain, options) {
    if (request === '@ohos/hvigor' || request.startsWith('@ohos/hvigor/')) {
      const suffix = request.slice('@ohos/hvigor'.length);
      request = suffix.length === 0 ? engineRoot : path.join(engineRoot, suffix.slice(1));
    } else if (request === '@ohos/hvigor-ohos-plugin' || request.startsWith('@ohos/hvigor-ohos-plugin/')) {
      const suffix = request.slice('@ohos/hvigor-ohos-plugin'.length);
      request = suffix.length === 0 ? pluginRoot : path.join(pluginRoot, suffix.slice(1));
    }
    return originalResolveFilename.call(this, request, parent, isMain, options);
  };
}
