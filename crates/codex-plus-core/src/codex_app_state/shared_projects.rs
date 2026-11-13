use std::path::Path;

use anyhow::{Context, ensure};
use serde_json::{Map, Value, json};

pub(super) const KEY: &str = "codex-plus-shared-local-projects-v1";

fn object(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn array(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && !["__proto__", "constructor", "prototype"].contains(&id)
        && !id.starts_with("linked:")
}

fn local_project(id: &str, value: &Value) -> bool {
    valid_id(id)
        && value.get("id").and_then(Value::as_str) == Some(id)
        && value
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
        && value.get("createdAt").is_some_and(Value::is_number)
        && value.get("updatedAt").is_some_and(Value::is_number)
        && value
            .get("rootPaths")
            .and_then(Value::as_array)
            .is_some_and(|roots| {
                !roots.is_empty()
                    && roots.iter().all(|root| {
                        root.as_str().is_some_and(|root| {
                            !root.contains("://")
                                && (root.starts_with('/')
                                    || root.starts_with("\\\\")
                                    || (root
                                        .as_bytes()
                                        .first()
                                        .is_some_and(u8::is_ascii_alphabetic)
                                        && root.as_bytes().get(1) == Some(&b':')
                                        && root
                                            .as_bytes()
                                            .get(2)
                                            .is_some_and(|c| [b'/', b'\\'].contains(c))))
                        })
                    })
            })
        && value.as_object().is_some_and(|fields| {
            fields.iter().all(|(key, value)| {
                let key = key.to_ascii_lowercase();
                value.is_null()
                    || !["chatgpt", "backing", "remote", "durable", "origin"]
                        .iter()
                        .any(|word| key.contains(word))
            })
        })
}

fn normalize_home(home: &Path) -> String {
    let path = home.to_string_lossy().replace('\\', "/");
    let path = path
        .strip_prefix("//?/")
        .unwrap_or(&path)
        .trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    if path.as_bytes().get(1) == Some(&b':') || path.starts_with("//") {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}

fn merge_source(record: &mut Value, globals: &Map<String, Value>, active: bool) {
    let projects = object(globals.get("local-projects"));
    let hosts = object(globals.get("thread-project-membership-host-ids"));
    for (id, project) in &projects {
        if !local_project(id, project) {
            continue;
        }
        let current = &record["projects"][id];
        let newer = project["updatedAt"].as_f64().unwrap_or(0.0)
            >= current["updatedAt"].as_f64().unwrap_or(0.0);
        if active || current.is_null() || newer {
            // Use the same small schema as the native notification companion.
            record["projects"][id] = json!({
                "id": id, "name": project["name"], "rootPaths": project["rootPaths"],
                "createdAt": project["createdAt"], "updatedAt": project["updatedAt"],
            });
        }
    }
    for (id, assignment) in object(globals.get("thread-project-assignments")) {
        let Some(project_id) = assignment["projectId"].as_str() else {
            continue;
        };
        if !valid_id(&id)
            || assignment["projectKind"] != "local"
            || !assignment["projectOrigin"].is_null()
            || hosts.get(&id).is_some_and(|host| host != "local")
            || !projects
                .get(project_id)
                .is_some_and(|p| local_project(project_id, p))
        {
            if active {
                record["memberships"].as_object_mut().unwrap().remove(&id);
            }
            continue;
        }
        if active || record["memberships"].get(&id).is_none() {
            record["memberships"][&id] = json!({
                "threadId": id, "hostId": "local", "projectless": false,
                "assignment": {"projectKind": "local", "projectId": project_id},
            });
        }
    }
    for id in array(globals.get("projectless-thread-ids"))
        .iter()
        .filter_map(Value::as_str)
    {
        if valid_id(id)
            && hosts.get(id).is_some_and(|host| host == "local")
            && globals
                .get("thread-project-assignments")
                .and_then(|a| a.get(id))
                .is_none()
        {
            record["memberships"][id] = json!({
                "threadId": id, "hostId": "local", "assignment": null, "projectless": true,
            });
        }
    }
    if active {
        let assignments = object(globals.get("thread-project-assignments"));
        let projectless = array(globals.get("projectless-thread-ids"));
        record["memberships"]
            .as_object_mut()
            .unwrap()
            .retain(|id, _| !hosts.get(id).is_some_and(|host| host != "local"));
        for (id, entry) in record["memberships"].as_object_mut().unwrap() {
            if !assignments.contains_key(id)
                && (projectless.contains(&json!(id))
                    || hosts.get(id).is_some_and(|host| host == "local"))
            {
                *entry = json!({
                    "threadId": id, "hostId": "local", "assignment": null,
                    "projectless": projectless.contains(&json!(id)),
                });
            }
        }
    }
    let mut order = array(globals.get("project-order"));
    order.extend(array(record.get("order")));
    let mut seen = std::collections::HashSet::new();
    order.retain(|id| {
        id.as_str()
            .is_some_and(|id| record["projects"].get(id).is_some() && seen.insert(id.to_string()))
    });
    record["order"] = json!(order);
}

pub(super) fn prepare(
    home: &Path,
    state: &Map<String, Value>,
    detached: Option<&Value>,
) -> anyhow::Result<Value> {
    if let Some(saved) = state.get(KEY) {
        validate(home, saved)?;
        let mut saved = saved.clone();
        // A final scope is authoritative only after this companion restored that scope.
        // This also captures a last native deletion whose old window was already retired.
        if saved["preparedForLaunch"] != true
            && let Some(owner) = saved["lastScope"]["owner"].as_str()
        {
            let container = state.get(super::ACCOUNT_RENDERER_STATE_KEY);
            let final_globals = if container.and_then(|v| v["owner"].as_str()) == Some(owner) {
                Some(state.clone())
            } else {
                container
                    .and_then(|v| v["inactive"][owner]["globals"].as_object())
                    .cloned()
            };
            if let Some(globals) = final_globals {
                let observed = array(saved["lastScope"].get("projects"));
                let before = object(saved.get("projects"));
                let observed_memberships = array(saved["lastScope"].get("memberships"));
                let previous_memberships = object(saved.get("memberships"));
                merge_source(&mut saved, &globals, true);
                for (id, entry) in previous_memberships {
                    if !observed_memberships.contains(&json!(id)) {
                        saved["memberships"][&id] = entry;
                    }
                }
                let hosts = object(globals.get("thread-project-membership-host-ids"));
                let projectless = array(globals.get("projectless-thread-ids"));
                for id in array(saved["lastScope"].get("memberships"))
                    .iter()
                    .filter_map(Value::as_str)
                {
                    if globals
                        .get("thread-project-assignments")
                        .and_then(|v| v.get(id))
                        .is_none()
                        && !hosts.get(id).is_some_and(|host| host != "local")
                    {
                        saved["memberships"][id] = json!({
                            "threadId":id, "hostId":"local", "assignment":null,
                            "projectless":projectless.contains(&json!(id)),
                        });
                    }
                }
                for id in observed.iter().filter_map(Value::as_str) {
                    if globals
                        .get("local-projects")
                        .and_then(|v| v.get(id))
                        .is_none()
                    {
                        saved["projects"].as_object_mut().unwrap().remove(id);
                    }
                }
                let known = array(saved.get("knownProjects"));
                saved["projects"]
                    .as_object_mut()
                    .unwrap()
                    .retain(|id, _| !known.contains(&json!(id)) || before.contains_key(id));
                let projects = object(saved.get("projects"));
                for (id, entry) in saved["memberships"].as_object_mut().unwrap() {
                    if entry["assignment"]["projectId"]
                        .as_str()
                        .is_some_and(|p| !projects.contains_key(p))
                    {
                        *entry = json!({"threadId": id, "hostId":"local", "assignment": null, "projectless":true});
                    }
                }
                for id in projects.keys() {
                    if !saved["knownProjects"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(id))
                    {
                        saved["knownProjects"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(id));
                    }
                }
            }
        }
        saved["enabled"] = json!(true);
        saved["preparedForLaunch"] = json!(true);
        if &saved != state.get(KEY).unwrap() {
            saved["revision"] = json!(saved["revision"].as_u64().unwrap() + 1);
        }
        return Ok(saved);
    }
    let mut record = json!({
        "version": 1, "home": normalize_home(home), "revision": 1,
        "enabled": true, "preparedForLaunch": true,
        "projects": {}, "memberships": {}, "knownProjects": [], "order": [],
    });
    for container in detached
        .into_iter()
        .chain(state.get(super::ACCOUNT_RENDERER_STATE_KEY))
    {
        for scope in object(container.get("inactive")).values() {
            let globals = scope
                .get("globals")
                .and_then(Value::as_object)
                .context("unrecognized inactive local project state")?;
            merge_source(&mut record, globals, false);
        }
    }
    merge_source(&mut record, state, true);
    let allowed = object(record.get("projects"));
    record["memberships"]
        .as_object_mut()
        .unwrap()
        .retain(|_, entry| {
            entry["assignment"].is_null()
                || entry["assignment"]["projectId"]
                    .as_str()
                    .is_some_and(|id| allowed.contains_key(id))
        });
    record["knownProjects"] = json!(allowed.keys().collect::<Vec<_>>());
    record["backup"] = json!({
        "projects": record["projects"], "memberships": record["memberships"], "order": record["order"],
    });
    Ok(record)
}

pub(super) fn validate(home: &Path, saved: &Value) -> anyhow::Result<()> {
    ensure!(
        saved["version"] == 1
            && saved["home"].as_str() == Some(normalize_home(home).as_str())
            && saved["revision"]
                .as_u64()
                .is_some_and(|n| n > 0 && n < 9_007_199_254_740_991)
            && saved["projects"].is_object()
            && saved["memberships"].is_object()
            && saved["order"].is_array()
            && saved["knownProjects"].is_array(),
        "unrecognized shared local project record; refusing to replace it"
    );
    ensure!(
        saved["projects"]
            .as_object()
            .unwrap()
            .iter()
            .all(|(id, p)| local_project(id, p)),
        "unrecognized shared local project metadata"
    );
    ensure!(
        saved["knownProjects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().is_some_and(valid_id))
            && saved["projects"]
                .as_object()
                .unwrap()
                .keys()
                .all(|id| saved["knownProjects"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(id)))
            && saved["memberships"]
                .as_object()
                .unwrap()
                .iter()
                .all(|(id, entry)| {
                    valid_id(id)
                        && entry["threadId"].as_str() == Some(id)
                        && entry["hostId"] == "local"
                        && entry["projectless"].is_boolean()
                        && (entry["assignment"].is_null()
                            || (entry["assignment"]["projectKind"] == "local"
                                && entry["assignment"]["projectOrigin"].is_null()
                                && entry["assignment"]["projectId"]
                                    .as_str()
                                    .is_some_and(|id| saved["projects"].get(id).is_some())))
                }),
        "unrecognized shared local project classifications"
    );
    ensure!(
        serde_json::to_vec(saved)?.len() <= 4 * 1024 * 1024,
        "shared local project record is too large"
    );
    Ok(())
}

pub(super) fn apply(state: &mut Map<String, Value>, record: &Value) {
    let mut projects = object(state.get("local-projects"));
    let known = array(record.get("knownProjects"));
    for id in known.iter().filter_map(Value::as_str) {
        if projects.get(id).is_some_and(|p| !local_project(id, p)) {
            continue;
        }
        if let Some(project) = record["projects"].get(id) {
            projects.insert(id.to_string(), project.clone());
        } else {
            projects.remove(id);
        }
    }
    let mut order = array(record.get("order"));
    for id in array(state.get("project-order")) {
        if !order.contains(&id) && id.as_str().is_some_and(|id| projects.contains_key(id)) {
            order.push(id);
        }
    }
    order.retain(|id| id.as_str().is_some_and(|id| projects.contains_key(id)));
    let mut assignments = object(state.get("thread-project-assignments"));
    let mut hosts = object(state.get("thread-project-membership-host-ids"));
    let mut projectless = array(state.get("projectless-thread-ids"));
    assignments.retain(|id, assignment| {
        let deleted = !hosts.get(id).is_some_and(|host| host != "local")
            && assignment["projectKind"] == "local"
            && assignment["projectOrigin"].is_null()
            && assignment["projectId"].as_str().is_some_and(|p| {
                known.contains(&json!(p))
                    && record["projects"].get(p).is_none()
                    && !projects.get(p).is_some_and(|v| !local_project(p, v))
            });
        if deleted && !projectless.contains(&json!(id)) {
            projectless.push(json!(id));
        }
        !deleted
    });
    for (id, entry) in object(record.get("memberships")) {
        if hosts.get(&id).is_some_and(|host| host != "local") {
            continue;
        }
        if assignments.get(&id).is_some_and(|a| {
            a["projectKind"] != "local"
                || !a["projectOrigin"].is_null()
                || a["projectId"]
                    .as_str()
                    .is_some_and(|id| projects.get(id).is_some_and(|p| !local_project(id, p)))
        }) {
            continue;
        }
        let assignment = &entry["assignment"];
        if let Some(project_id) = assignment["projectId"].as_str() {
            if !projects
                .get(project_id)
                .is_some_and(|p| local_project(project_id, p))
            {
                continue;
            }
            assignments.insert(id.clone(), assignment.clone());
        } else {
            assignments.remove(&id);
        }
        projectless.retain(|value| value.as_str() != Some(&id));
        if entry["projectless"] == true {
            projectless.push(json!(id));
        }
        hosts.insert(id, json!("local"));
    }
    state.insert("local-projects".to_string(), json!(projects));
    state.insert("project-order".to_string(), json!(order));
    state.insert("thread-project-assignments".to_string(), json!(assignments));
    state.insert(
        "thread-project-membership-host-ids".to_string(),
        json!(hosts),
    );
    state.insert("projectless-thread-ids".to_string(), json!(projectless));
    if let Some(pinned) = state
        .get_mut("pinned-project-ids")
        .and_then(Value::as_array_mut)
    {
        pinned.retain(|id| {
            !known.contains(id) || id.as_str().is_some_and(|id| projects.contains_key(id))
        });
    }
    if state.get("selected-project").is_some_and(|id| {
        known.contains(id) && id.as_str().is_some_and(|id| !projects.contains_key(id))
    }) {
        state.remove("selected-project");
    }
    state.insert(KEY.to_string(), record.clone());
}
