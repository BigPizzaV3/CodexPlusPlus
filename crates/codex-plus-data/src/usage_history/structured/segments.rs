use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

pub(super) fn causal_order(
    events: &mut Vec<super::UsageEvent>,
    files: &BTreeMap<PathBuf, super::FileState>,
) -> (Vec<usize>, bool) {
    let selected: HashMap<_, _> = events
        .iter()
        .enumerate()
        .map(|(position, event)| (event.id, position))
        .collect();
    let mut successors = vec![Vec::new(); events.len()];
    for state in files.values() {
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
    (components, cyclic)
}

fn root(parents: &mut [usize], mut position: usize) -> usize {
    while parents[position] != position {
        parents[position] = parents[parents[position]];
        position = parents[position];
    }
    position
}
