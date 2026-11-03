// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-session 替身。
// 插件多半只拿品牌类型的校验函数与常量；会话本身由宿主注入的 ctx.sessions 提供（Codex 会话的镜像，见 services.mjs）。

export const SESSION_FORMAT_VERSION = 4
export const KNOWN_SESSION_EVENT_TYPES = Object.freeze([
  'user/message', 'assistant/message', 'assistant/attempt', 'developer/message', 'system/message',
  'tool/call', 'tool/result', 'turn/start', 'turn/end', 'step/start', 'step/end',
  'compaction/start', 'compaction/summary', 'compaction/end', 'session/title', 'session/end-seed',
])

const brand = (name, check) => (value) => {
  if (!check(value)) throw new TypeError(`invalid ${name}: ${String(value)}`)
  return value
}
export const SessionId = brand('SessionId', (value) => typeof value === 'string' && value.length > 0)
export const SessionSeq = brand('SessionSeq', (value) => Number.isInteger(value) && value >= 0)
export const SessionLogOffset = brand('SessionLogOffset', (value) => Number.isInteger(value) && value >= 0)

const MESSAGE_TYPES = new Set(['system/message', 'developer/message', 'user/message', 'assistant/message', 'tool/result'])
export const isSurfaceEvent = (event) => MESSAGE_TYPES.has(event?.type)
export const isAppendSurfaceEvent = (event) => isSurfaceEvent(event) && (event.surfaceOp === undefined || event.surfaceOp === 'append')
export const isReplacementSurfaceEvent = (event) => isSurfaceEvent(event) && typeof event.surfaceOp === 'object'
export const headerEquals = (a, b) => JSON.stringify(a) === JSON.stringify(b)
export const canonicalHeader = (header) => ({ version: SESSION_FORMAT_VERSION, isSeeded: false, ...header })
export const snapshotSessionEvent = (event) => structuredClone(event)

/** 会话事件里的消息（消息类事件才有）。 */
export function deriveEventMessage(event) {
  if (event?.type === 'user/message') return event.data
  if (MESSAGE_TYPES.has(event?.type)) return event.data?.message
  return undefined
}

export class Session {
  constructor() {
    throw new Error('Session is provided by the host as ctx.sessions in Codex++')
  }
}

/* Derived from @deepseek-ai/dsh-session@0.0.1-rc.1, lib/types/json.js (BSD-3-Clause). Redistributed under its license, reproduced below.
 *
 * BSD 3-Clause License
 * 
 * Copyright (c) 2026, DeepSeek
 * 
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 * 
 * 1. Redistributions of source code must retain the above copyright notice, this
 *    list of conditions and the following disclaimer.
 * 
 * 2. Redistributions in binary form must reproduce the above copyright notice,
 *    this list of conditions and the following disclaimer in the documentation
 *    and/or other materials provided with the distribution.
 * 
 * 3. Neither the name of the copyright holder nor the names of its
 *    contributors may be used to endorse or promote products derived from
 *    this software without specific prior written permission.
 * 
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
 * FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
 * DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
 * SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
 * CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
 * OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 * OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

/** Lossless-JSON validation and detached snapshots for durable session data. @module @deepseek-ai/dsh-session/json */
/** Whether a realm-owned intrinsic prototype is backed by its native constructor. */
function hasIntrinsicConstructor(prototype, name) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, 'constructor');
    const constructor = descriptor?.value;
    if (typeof constructor !== 'function')
        return false;
    try {
        return constructor.name === name
            && constructor.prototype === prototype
            && Function.prototype.toString.call(constructor) === `function ${name}() { [native code] }`;
    }
    catch {
        return false;
    }
}
/** Whether a candidate is one realm's intrinsic `Object.prototype`. */
function isIntrinsicObjectPrototype(value) {
    return Object.getPrototypeOf(value) === null && hasIntrinsicConstructor(value, 'Object');
}
/** Whether an array uses one realm's intrinsic `Array.prototype`, not a subclass or forged prototype. */
function hasPlainArrayPrototype(value) {
    const prototype = Object.getPrototypeOf(value);
    if (!Array.isArray(prototype) || !hasIntrinsicConstructor(prototype, 'Array'))
        return false;
    const objectPrototype = Object.getPrototypeOf(prototype);
    return typeof objectPrototype === 'object'
        && objectPrototype !== null
        && isIntrinsicObjectPrototype(objectPrototype);
}
/** Whether an object is a plain or null-prototype record from any JavaScript realm. */
function hasPlainObjectPrototype(value) {
    const prototype = Object.getPrototypeOf(value);
    return prototype === null
        || typeof prototype === 'object' && isIntrinsicObjectPrototype(prototype);
}
/** Return every JSON-visible object key, or reject own data JSON would discard. */
function enumerableStringKeys(value) {
    const keys = Reflect.ownKeys(value);
    if (keys.some(key => typeof key !== 'string' || !Object.prototype.propertyIsEnumerable.call(value, key)))
        return undefined;
    return keys;
}
/** Validate lossless JSON iteratively, optionally materializing a detached snapshot. */
function walkJsonValue(value, detach) {
    const ancestors = new Set();
    let root;
    const assign = (destination, item) => {
        if (destination === undefined)
            return;
        if (destination.kind === 'root') {
            root = item;
        }
        else if (destination.kind === 'array') {
            destination.target[destination.index] = item;
        }
        else {
            Object.defineProperty(destination.target, destination.key, {
                value: item,
                enumerable: true,
                configurable: true,
                writable: true,
            });
        }
    };
    const tasks = [{
            kind: 'visit',
            value,
            ...(detach ? { destination: { kind: 'root' } } : {}),
        }];
    for (let task = tasks.pop(); task !== undefined; task = tasks.pop()) {
        if (task.kind === 'leave') {
            ancestors.delete(task.source);
            continue;
        }
        if (task.kind === 'array-item') {
            if (!Object.prototype.hasOwnProperty.call(task.source, task.index))
                return undefined;
            tasks.push({
                kind: 'visit',
                value: task.source[task.index],
                ...(task.target === undefined ? {} : { destination: { kind: 'array', target: task.target, index: task.index } }),
            });
            continue;
        }
        if (task.kind === 'object-property') {
            tasks.push({
                kind: 'visit',
                value: task.source[task.key],
                ...(task.target === undefined ? {} : { destination: { kind: 'object', target: task.target, key: task.key } }),
            });
            continue;
        }
        const current = task.value;
        if (current === null) {
            assign(task.destination, null);
            continue;
        }
        if (typeof current === 'boolean' || typeof current === 'string') {
            assign(task.destination, current);
            continue;
        }
        if (typeof current === 'number') {
            if (!Number.isFinite(current) || Object.is(current, -0))
                return undefined;
            assign(task.destination, current);
            continue;
        }
        if (typeof current !== 'object')
            return undefined;
        if (ancestors.has(current))
            return undefined;
        if (Array.isArray(current)) {
            if (!hasPlainArrayPrototype(current))
                return undefined;
            const length = current.length;
            if (Reflect.ownKeys(current).length !== length + 1)
                return undefined;
            const target = detach ? [] : undefined;
            if (target !== undefined)
                assign(task.destination, target);
            ancestors.add(current);
            tasks.push({ kind: 'leave', source: current });
            for (let index = length - 1; index >= 0; index--) {
                tasks.push({ kind: 'array-item', source: current, index, ...(target === undefined ? {} : { target }) });
            }
            continue;
        }
        if (!hasPlainObjectPrototype(current))
            return undefined;
        const keys = enumerableStringKeys(current);
        if (keys === undefined)
            return undefined;
        const target = detach ? {} : undefined;
        if (target !== undefined)
            assign(task.destination, target);
        ancestors.add(current);
        tasks.push({ kind: 'leave', source: current });
        for (let index = keys.length - 1; index >= 0; index--) {
            const key = keys[index];
            /* v8 ignore next -- the loop is bounded by the captured key count. */
            if (key === undefined)
                return undefined;
            tasks.push({ kind: 'object-property', source: current, key, ...(target === undefined ? {} : { target }) });
        }
    }
    return detach ? root : true;
}
/**
 * Validate and detach lossless JSON in one read per property, so a stateful
 * getter cannot change between validation and copying. Traversal is iterative,
 * so valid nesting is bounded by available memory rather than the JavaScript
 * call stack. Accepts ordinary arrays, plain or null-prototype objects, and JSON
 * scalars; rejects sparse, cyclic, exotic, negative-zero, and non-finite values.
 * Getter throws propagate.
 *
 * @param value - the candidate value to validate and detach.
 * @returns the detached snapshot, or `undefined` when the value is not
 *   losslessly JSON-serializable.
 */
export function snapshotJsonValue(value) {
    return walkJsonValue(value, true);
}
/**
 * Test the same lossless JSON boundary as {@link snapshotJsonValue} without
 * detaching it. Only own enumerable string properties participate; `toJSON`
 * is ignored and getters run, so persistence boundaries use the snapshotter.
 * @param value - the candidate event data to test.
 * @returns whether `value` survives JSON round-trip losslessly.
 */
export function isJsonValue(value) {
    return walkJsonValue(value, false) === true;
}
