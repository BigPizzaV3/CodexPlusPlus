  /**
   * 四块布局的纯几何求解器。每次从拖动开始的基线求解，不读 DOM，不积累动画误差。
   * 活跃块优先；其它块沿相邻边缘让位，必要时才收敛尺寸。搜索有固定预算。
   */
  function codexPlusLayoutNumber(value, fallback = 0) {
    return typeof value === "number" && Number.isFinite(value) ? value : fallback;
  }

  function codexPlusLayoutLimit(value, minimum, maximum) {
    return Math.max(minimum, Math.min(maximum, value));
  }

  function codexPlusLayoutArea(bounds) {
    const left = codexPlusLayoutNumber(bounds?.left);
    const top = codexPlusLayoutNumber(bounds?.top);
    return {
      left, top,
      width: Math.max(1, codexPlusLayoutNumber(bounds?.width, codexPlusLayoutNumber(bounds?.right, left + 1) - left)),
      height: Math.max(1, codexPlusLayoutNumber(bounds?.height, codexPlusLayoutNumber(bounds?.bottom, top + 1) - top)),
    };
  }

  function codexPlusLayoutRect(value, area, minimum = {}) {
    const width = codexPlusLayoutLimit(codexPlusLayoutNumber(value?.width, 40), Math.min(area.width, minimum.width || 1), area.width);
    const height = codexPlusLayoutLimit(codexPlusLayoutNumber(value?.height, 24), Math.min(area.height, minimum.height || 1), area.height);
    return {
      left: codexPlusLayoutLimit(codexPlusLayoutNumber(value?.left, area.left), area.left, area.left + area.width - width),
      top: codexPlusLayoutLimit(codexPlusLayoutNumber(value?.top, area.top), area.top, area.top + area.height - height),
      width, height,
    };
  }

  function codexPlusLayoutOverlap(a, b) {
    const epsilon = 0.000001;
    return a.left < b.left + b.width - epsilon && a.left + a.width > b.left + epsilon
      && a.top < b.top + b.height - epsilon && a.top + a.height > b.top + epsilon;
  }

  function codexPlusLayoutSame(a, b) {
    return ["left", "top", "width", "height"].every((key) => Math.abs(a[key] - b[key]) < 0.0001);
  }

  function codexPlusLayoutCost(rect, base) {
    const dx = rect.left - base.left;
    const dy = rect.top - base.top;
    const dw = rect.width - base.width;
    const dh = rect.height - base.height;
    // 小幅移动比改变尺寸便宜；为了几像素碰撞横跳整屏则比小幅收敛尺寸贵。
    return dx * dx + dy * dy + 12 * (dw * dw + dh * dh);
  }

  function codexPlusLayoutSizes(original, minimum, available, compress) {
    if (!compress || original <= minimum) return [original];
    const values = [original];
    const free = [...new Set(available.filter((value) => Number.isFinite(value) && value >= minimum && value < original))]
      .sort((a, b) => b - a);
    values.push(...free.slice(0, 3));
    if (!values.some((value) => Math.abs(value - minimum) < 0.0001)) values.push(minimum);
    return values;
  }

  function codexPlusLayoutCandidates(panel, placed, baselines, area, compress, budget) {
    const base = panel.rect;
    const right = area.left + area.width;
    const bottom = area.top + area.height;
    const widths = [], heights = [];
    const anchorKeys = new Set();
    const anchors = [...placed, ...baselines.filter((other) => other.id !== panel.id).map((other) => other.rect)]
      .filter((rect) => {
        const key = [rect.left, rect.top, rect.width, rect.height].join(",");
        if (anchorKeys.has(key)) return false;
        anchorKeys.add(key);
        return true;
      });
    // 尚未排入的原位边缘也提供空隙候选。例如输入框可以收窄到
    // 「图标栏右沿 → 正在拖动的聊天栏左沿」，而不用先把图标栏挤走。
    // 这些原位只是搜索锚点，碰撞仍仅以已排入的块为硬约束。
    for (const obstacle of anchors) {
      widths.push(obstacle.left - area.left, right - obstacle.left - obstacle.width,
        obstacle.left - base.left, right - Math.max(base.left, obstacle.left + obstacle.width));
      heights.push(obstacle.top - area.top, bottom - obstacle.top - obstacle.height,
        obstacle.top - base.top, bottom - Math.max(base.top, obstacle.top + obstacle.height));
    }
    for (const first of anchors) {
      for (const second of anchors) {
        widths.push(second.left - first.left - first.width);
        heights.push(second.top - first.top - first.height);
      }
    }
    const widthOptions = codexPlusLayoutSizes(base.width, panel.minWidth, widths, compress);
    const heightOptions = codexPlusLayoutSizes(base.height, panel.minHeight, heights, compress && panel.canResizeHeight);
    const candidates = [];
    const seen = new Set();
    for (const width of widthOptions) {
      for (const height of heightOptions) {
        const xs = [base.left, area.left, right - width];
        const ys = [base.top, area.top, bottom - height];
        for (const obstacle of anchors) {
          xs.push(obstacle.left - width, obstacle.left + obstacle.width, obstacle.left, obstacle.left + obstacle.width - width);
          ys.push(obstacle.top - height, obstacle.top + obstacle.height, obstacle.top, obstacle.top + obstacle.height - height);
        }
        // 单轴推出和近邻交点都参与搜索，既允许连锁挤开，也允许在空位回填。
        for (const x of [...new Set(xs)]) {
          for (const y of [...new Set(ys)]) {
            if (++budget.candidates > budget.candidateLimit) return candidates.sort((a, b) => a.cost - b.cost).slice(0, 18);
            if (x < area.left - 0.000001 || x + width > right + 0.000001 || y < area.top - 0.000001 || y + height > bottom + 0.000001) continue;
            const rect = { left: Math.max(area.left, x), top: Math.max(area.top, y), width, height };
            const key = [rect.left, rect.top, width, height].map((value) => value.toFixed(5)).join(",");
            if (seen.has(key) || placed.some((obstacle) => codexPlusLayoutOverlap(rect, obstacle))) continue;
            seen.add(key);
            candidates.push({ rect, cost: codexPlusLayoutCost(rect, base) });
          }
        }
      }
    }
    return candidates.sort((a, b) => a.cost - b.cost || b.rect.width - a.rect.width || b.rect.height - a.rect.height
      || a.rect.left - b.rect.left || a.rect.top - b.rect.top).slice(0, 18);
  }

  function codexPlusLayoutPack(panels, activeId, activeRect, area, compress, budget) {
    let best = null;
    let bestCost = Infinity;
    const initial = { [activeId]: activeRect };
    const remaining = panels.filter((panel) => panel.id !== activeId);
    const search = (unplaced, rects, cost) => {
      if (++budget.nodes > budget.nodeLimit || budget.candidates > budget.candidateLimit || cost >= bestCost) return;
      if (!unplaced.length) {
        best = { ...rects };
        bestCost = cost;
        return;
      }
      const occupied = Object.values(rects);
      // 先处理直接碰撞的块，再沿已推出的块传播。相同条件按 id 固定排序。
      const sorted = [...unplaced].sort((a, b) => {
        const aHits = occupied.filter((rect) => codexPlusLayoutOverlap(a.rect, rect)).length;
        const bHits = occupied.filter((rect) => codexPlusLayoutOverlap(b.rect, rect)).length;
        return bHits - aHits || a.id.localeCompare(b.id);
      });
      const panel = sorted[0];
      const next = sorted.slice(1);
      for (const candidate of codexPlusLayoutCandidates(panel, occupied, panels, area, compress, budget)) {
        if (cost + candidate.cost >= bestCost) continue;
        search(next, { ...rects, [panel.id]: candidate.rect }, cost + candidate.cost);
      }
    };
    search(remaining, initial, 0);
    return best ? { rects: best, cost: bestCost } : null;
  }

  function codexPlusLayoutSolve(values, activeId, target, bounds, origin = null) {
    const area = codexPlusLayoutArea(bounds);
    const input = Array.isArray(values) ? values.map((value) => [value?.id, value]) : Object.entries(values || {});
    const byId = new Map();
    for (const [key, value] of input) {
      if (typeof key !== "string" || !key || !value || ![value.left, value.top, value.width, value.height].every(Number.isFinite)) continue;
      const minWidth = Math.max(1, Math.min(area.width, codexPlusLayoutNumber(value.minWidth, Math.min(value.width, 80))));
      const canResizeHeight = value.canResizeHeight === true;
      const minHeight = canResizeHeight ? Math.max(1, Math.min(area.height, codexPlusLayoutNumber(value.minHeight, Math.min(value.height, 80)))) : Math.min(value.height, area.height);
      byId.set(key, { id: key, minWidth, minHeight, canResizeHeight,
        original: { left: value.left, top: value.top, width: value.width, height: value.height },
        rect: codexPlusLayoutRect(value, area, { width: minWidth, height: minHeight }) });
    }
    // 运行时只管理四块；更大输入也限制搜索规模，不把单帧变成无界装箱问题。
    const panels = [...byId.values()].sort((a, b) => a.id.localeCompare(b.id));
    const baseline = Object.fromEntries(panels.map((panel) => [panel.id, { ...panel.rect }]));
    const active = byId.get(activeId);
    const result = (rects, blocked, feasible = true) => ({
      rects,
      changedIds: panels.filter((panel) => rects[panel.id] && !codexPlusLayoutSame(rects[panel.id], panel.original)).map((panel) => panel.id),
      blocked,
      feasible,
    });
    if (!active || panels.length > 4) return result(baseline, true, false);
    const requested = codexPlusLayoutRect({ ...active.rect, ...target }, area, { width: active.minWidth, height: active.minHeight });
    const boundaryLimited = target != null && ["left", "top", "width", "height"].some((key) => Number.isFinite(target[key]) && Math.abs(target[key] - requested[key]) > 0.0001);
    const budget = { nodes: 0, candidates: 0, nodeLimit: 1800, candidateLimit: 50000 };
    const attempt = (rect) => {
      const preserved = codexPlusLayoutPack(panels, activeId, rect, area, false, budget);
      if (preserved && preserved.cost <= 24 * 24) return preserved;
      const compressed = codexPlusLayoutPack(panels, activeId, rect, area, true, budget);
      return compressed && (!preserved || compressed.cost < preserved.cost) ? compressed : preserved;
    };
    // 原位回填走零成本路径：回到基线就恢复其它块，不继承上一帧挤开的偏移。
    const full = attempt(requested);
    if (full) return result(full.rects, boundaryLimited);
    const start = codexPlusLayoutRect(origin || active.rect, area, { width: active.minWidth, height: active.minHeight });
    const baselineValid = panels.every((panel, index) => panels.slice(index + 1).every((other) => !codexPlusLayoutOverlap(panel.rect, other.rect)));
    let last = codexPlusLayoutSame(start, active.rect) && baselineValid ? { rects: baseline } : attempt(start);
    if (!last) return result(baseline, true, false);
    let low = 0, high = 1;
    for (let iteration = 0; iteration < 10 && budget.nodes < budget.nodeLimit && budget.candidates < budget.candidateLimit; iteration += 1) {
      const progress = (low + high) / 2;
      const probe = {};
      for (const key of ["left", "top", "width", "height"]) probe[key] = start[key] + (requested[key] - start[key]) * progress;
      const packed = attempt(probe);
      if (packed) {
        low = progress;
        last = packed;
      } else high = progress;
    }
    return result(last.rects, true);
  }

  /** 解析阻尼弹簧：dt 以秒计，分帧方式不会改变同一实际时间的运动结果。 */
  function codexPlusLayoutSpringStep(position, velocity, target, dt, options = {}) {
    const minimum = codexPlusLayoutNumber(options.min, -Infinity);
    const maximum = Math.max(minimum, codexPlusLayoutNumber(options.max, Infinity));
    const sourcePosition = codexPlusLayoutNumber(position);
    const destination = codexPlusLayoutLimit(codexPlusLayoutNumber(target, sourcePosition), minimum, maximum);
    const current = codexPlusLayoutLimit(codexPlusLayoutNumber(position, destination), minimum, maximum);
    const speed = codexPlusLayoutNumber(velocity);
    if (options.reducedMotion) return { position: destination, velocity: 0, settled: true };
    const seconds = Math.max(0, codexPlusLayoutNumber(dt));
    const stiffness = Math.max(0.0001, codexPlusLayoutNumber(options.stiffness, 260));
    const damping = Math.max(0, codexPlusLayoutNumber(options.damping, 21));
    const mass = Math.max(0.0001, codexPlusLayoutNumber(options.mass, 1));
    const omega = Math.sqrt(stiffness / mass);
    const decay = damping / (2 * mass);
    const displacement = current - destination;
    let nextPosition = current, nextVelocity = speed;
    if (seconds > 0) {
      if (decay < omega - 0.000001) {
        const frequency = Math.sqrt(omega * omega - decay * decay);
        const exponential = Math.exp(-decay * seconds);
        const cosine = Math.cos(frequency * seconds), sine = Math.sin(frequency * seconds);
        const coefficient = (speed + decay * displacement) / frequency;
        const offset = displacement * cosine + coefficient * sine;
        nextPosition = destination + exponential * offset;
        nextVelocity = exponential * (-decay * offset - displacement * frequency * sine + coefficient * frequency * cosine);
      } else if (Math.abs(decay - omega) <= 0.000001) {
        const exponential = Math.exp(-omega * seconds);
        const coefficient = speed + omega * displacement;
        nextPosition = destination + exponential * (displacement + coefficient * seconds);
        nextVelocity = exponential * (coefficient - omega * (displacement + coefficient * seconds));
      } else {
        const delta = Math.sqrt(decay * decay - omega * omega);
        const first = -decay + delta, second = -decay - delta;
        const a = (speed - second * displacement) / (first - second), b = displacement - a;
        const firstTerm = a * Math.exp(first * seconds), secondTerm = b * Math.exp(second * seconds);
        nextPosition = destination + firstTerm + secondTerm;
        nextVelocity = first * firstTerm + second * secondTerm;
      }
    }
    if (!Number.isFinite(nextPosition) || !Number.isFinite(nextVelocity)) return { position: destination, velocity: 0, settled: true };
    if (nextPosition < minimum || nextPosition > maximum) {
      nextPosition = codexPlusLayoutLimit(nextPosition, minimum, maximum);
      nextVelocity = 0;
    }
    const settled = Math.abs(nextPosition - destination) < 0.05 && Math.abs(nextVelocity) < 0.1;
    return settled ? { position: destination, velocity: 0, settled: true }
      : { position: nextPosition, velocity: nextVelocity, settled: false };
  }

  function springStep(position, velocity, target, dt, options = {}) {
    return codexPlusLayoutSpringStep(position, velocity, target, dt, options);
  }
