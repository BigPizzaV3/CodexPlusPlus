use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub(super) fn rollout_id(path: &Path) -> Option<uuid::Uuid> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_suffix(".zst")
        .unwrap_or(name)
        .strip_suffix(".jsonl")?;
    uuid::Uuid::parse_str(stem.get(stem.len().saturating_sub(36)..)?).ok()
}

pub(super) struct HistoryBase {
    rollout: uuid::Uuid,
    end_ordinal: u64,
    end_offset: u64,
}

impl HistoryBase {
    pub(super) fn parse(value: &serde_json::Value, ordinal: Option<u64>) -> Option<Self> {
        let end_ordinal = value["end_ordinal_exclusive"].as_u64()?;
        let end_offset = value["end_byte_offset"].as_u64()?;
        if end_ordinal == 0 || end_offset == 0 || ordinal != Some(end_ordinal) {
            return None;
        }
        Some(Self {
            rollout: uuid::Uuid::parse_str(value["thread_id"].as_str()?).ok()?,
            end_ordinal,
            end_offset,
        })
    }
}

pub(super) enum PrefixCount {
    Event([u8; 32], super::Tokens),
    Baseline(super::Tokens),
}

pub(super) fn referenced_prefixes(
    files: &BTreeMap<PathBuf, super::FileState>,
) -> BTreeMap<&PathBuf, PrefixCount> {
    if !files.values().any(|file| file.history_base.is_some()) {
        return BTreeMap::new();
    }
    // thread_id 指物理 rollout ID。只使用已扫描的本机会话，不从记录中读取任何路径。
    let mut rollouts = HashMap::new();
    for (path, state) in files {
        if let Some(rollout) = rollout_id(path) {
            rollouts
                .entry(rollout)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(state));
        }
    }
    fn count_before<'a>(
        mut base: &'a HistoryBase,
        rollouts: &HashMap<uuid::Uuid, Option<&'a super::FileState>>,
    ) -> Option<PrefixCount> {
        loop {
            let source = rollouts.get(&base.rollout)?.as_ref()?;
            let (start, last, parsed_offset) = source.rollout_extent?;
            if source.identity_conflict
                || source.invalid_counts
                || source.rollout_out_of_order
                || source.issues != 0
                || source
                    .history_base
                    .as_ref()
                    .map_or(start != 0, |parent| parent.end_ordinal != start)
                || start >= base.end_ordinal
                || last.saturating_add(1) < base.end_ordinal
                || parsed_offset < base.end_offset
                || last.saturating_add(1) == base.end_ordinal && parsed_offset != base.end_offset
            {
                return None;
            }
            let split = source.events.partition_point(|event| {
                event
                    .rollout_position
                    .is_some_and(|(ordinal, _, _)| ordinal < base.end_ordinal)
            });
            let previous = split.checked_sub(1).map(|i| &source.events[i]);
            let older = source.expired_count;
            if previous.is_some_and(|event| {
                event
                    .rollout_position
                    .is_none_or(|(_, _, end)| end > base.end_offset)
            }) || source.events.get(split).is_some_and(|event| {
                event
                    .rollout_position
                    .is_none_or(|(_, start, _)| start < base.end_offset)
            }) || older.is_some_and(|((ordinal, _, end), _)| {
                ordinal >= base.end_ordinal || end > base.end_offset
            }) {
                return None;
            }
            match (previous, older) {
                (Some(event), Some((position, total)))
                    if Some(position) > event.rollout_position && position.0 < base.end_ordinal =>
                {
                    return Some(PrefixCount::Baseline(total));
                }
                (Some(event), _) => return Some(PrefixCount::Event(event.id, event.total)),
                (None, Some((_, total))) => return Some(PrefixCount::Baseline(total)),
                (None, None) if !source.continuation => {
                    return Some(PrefixCount::Baseline(super::Tokens::default()));
                }
                (None, None) => {
                    base = source
                        .history_base
                        .as_ref()
                        .filter(|parent| parent.end_ordinal == start)?;
                }
            }
        }
    }
    files
        .iter()
        .filter_map(|(path, state)| {
            if state.invalid_counts
                || state.rollout_out_of_order
                || !state
                    .events
                    .first()
                    .is_some_and(|event| !event.has_file_baseline)
            {
                return None;
            }
            count_before(state.history_base.as_ref()?, &rollouts).map(|count| (path, count))
        })
        .collect()
}

pub(super) fn causal_order(
    events: &mut Vec<super::UsageEvent>,
    files: &BTreeMap<PathBuf, super::FileState>,
    prefixes: &BTreeMap<&PathBuf, PrefixCount>,
) -> (Vec<usize>, Vec<Vec<usize>>, bool) {
    let selected: HashMap<_, _> = events
        .iter()
        .enumerate()
        .map(|(position, event)| (event.id, position))
        .collect();
    let mut successors = vec![Vec::new(); events.len()];
    for (path, state) in files {
        if state.identity_conflict || state.session.is_empty() && state.stamp.is_some() {
            continue;
        }
        let mut previous = HashMap::new();
        for candidate in &state.events {
            let Some(&position) = selected.get(&candidate.id) else {
                continue;
            };
            if candidate.inherited != events[position].inherited {
                continue;
            }
            if let Some(before) = previous.insert(candidate.session.as_str(), position) {
                if before != position {
                    successors[before].push(position);
                }
            } else if !candidate.has_file_baseline
                && let Some(PrefixCount::Event(id, _)) = prefixes.get(path)
                && let Some(&before) = selected.get(id)
                && before != position
            {
                successors[before].push(position);
            }
        }
    }

    let mut parents: Vec<_> = (0..events.len()).collect();
    let mut indegree = vec![0usize; events.len()];
    for (before, afters) in successors.iter_mut().enumerate() {
        afters.sort_unstable();
        afters.dedup();
        for &after in afters.iter() {
            indegree[after] += 1;
            let left = root(&mut parents, before);
            let right = root(&mut parents, after);
            parents[left.max(right)] = left.min(right);
        }
    }

    let (order, resolved) = {
        let key = |position: usize| {
            let event = &events[position];
            (
                event.session.as_str(),
                event.timestamp.as_str(),
                &event.id,
                position,
            )
        };
        let mut ready = BTreeSet::new();
        for (position, &degree) in indegree.iter().enumerate() {
            if degree == 0 {
                ready.insert(key(position));
            }
        }
        let mut order = Vec::with_capacity(events.len());
        while let Some((_, _, _, position)) = ready.pop_first() {
            order.push(position);
            for &after in &successors[position] {
                indegree[after] -= 1;
                if indegree[after] == 0 {
                    ready.insert(key(after));
                }
            }
        }
        let resolved = order.len();
        let mut blocked: Vec<_> = (0..events.len())
            .filter(|&position| indegree[position] != 0)
            .collect();
        blocked.sort_unstable_by_key(|&position| key(position));
        order.extend(blocked);
        (order, resolved)
    };
    let cyclic = resolved != events.len();
    for &position in &order[resolved..] {
        events[position].count_uncertain = true;
    }

    let mut ordered_positions = vec![0; events.len()];
    for (position, &original) in order.iter().enumerate() {
        ordered_positions[original] = position;
    }
    let mut predecessors = vec![Vec::new(); events.len()];
    for (before, afters) in successors.iter().enumerate() {
        for &after in afters {
            predecessors[ordered_positions[after]].push(ordered_positions[before]);
        }
    }

    let mut component_ids = HashMap::new();
    let components = order
        .iter()
        .map(|&position| {
            let next = component_ids.len();
            *component_ids
                .entry(root(&mut parents, position))
                .or_insert(next)
        })
        .collect();
    let mut original: Vec<_> = events.drain(..).map(Some).collect();
    events.extend(
        order
            .into_iter()
            .map(|position| original[position].take().unwrap()),
    );
    (components, predecessors, cyclic)
}

pub(super) fn prune_transitive_predecessors(
    predecessors: &mut [Vec<usize>],
    components: &[usize],
    work_per_component: usize,
) -> Vec<bool> {
    let mut unresolved = vec![false; predecessors.len()];
    let mut visited = vec![0usize; predecessors.len()];
    let mut source_roots = vec![0usize; predecessors.len()];
    let mut stack = Vec::new();
    let mut budgets =
        vec![work_per_component; components.iter().copied().max().map_or(0, |n| n + 1)];
    for position in 0..predecessors.len() {
        let mut candidates = std::mem::take(&mut predecessors[position]);
        candidates.sort_unstable();
        candidates.dedup();
        source_roots[position] = candidates
            .first()
            .filter(|&&before| before < position)
            .map_or(position, |&before| source_roots[before]);
        if candidates.len() < 2 || candidates.iter().any(|&before| before >= position) {
            predecessors[position] = candidates;
            continue;
        }
        if candidates.len() == 2 && source_roots[candidates[1]] == candidates[0] {
            source_roots[position] = source_roots[candidates[1]];
            predecessors[position] = vec![candidates[1]];
            continue;
        }
        let epoch = position + 1;
        let minimum = candidates[0];
        let budget = &mut budgets[components[position]];
        let mut direct = Vec::with_capacity(candidates.len());
        let mut exhausted = false;
        for &candidate in candidates.iter().rev() {
            if visited[candidate] == epoch {
                continue;
            }
            direct.push(candidate);
            visited[candidate] = epoch;
            stack.push(candidate);
            while let Some(node) = stack.pop() {
                if node <= minimum {
                    continue;
                }
                for &before in &predecessors[node] {
                    if *budget == 0 {
                        exhausted = true;
                        break;
                    }
                    *budget -= 1;
                    if before >= minimum && visited[before] != epoch {
                        visited[before] = epoch;
                        stack.push(before);
                    }
                }
                if exhausted {
                    break;
                }
            }
            if exhausted {
                break;
            }
        }
        stack.clear();
        if exhausted {
            unresolved[position] = true;
            predecessors[position] = candidates;
        } else {
            direct.reverse();
            predecessors[position] = direct;
        }
    }
    unresolved
}

fn root(parents: &mut [usize], mut position: usize) -> usize {
    while parents[position] != position {
        parents[position] = parents[parents[position]];
        position = parents[position];
    }
    position
}
