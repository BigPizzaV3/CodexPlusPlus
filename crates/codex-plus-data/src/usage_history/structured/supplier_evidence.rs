use super::*;

const LIMIT: u64 = 32 * 1024 * 1024;
const SOURCE: &str = "upstream-counter-bidirectional-supplier-only";
const MAX_MODEL_DETAILS: usize = 128;

#[derive(Default)]
pub(super) struct Cache {
    signatures: HashMap<String, Option<[u8; 32]>>,
    upstream_owners: HashMap<String, Option<String>>,
    request_owners: HashMap<String, Option<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    records: Vec<SupplierRecord>,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupplierRecord {
    event: EventBinding,
    supplier_evidence: SupplierBinding,
    model_evidence: ModelBinding,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventBinding {
    event_id: String,
    session: String,
    timestamp: String,
    recorded_model: String,
    role: String,
    input: u64,
    output: u64,
    cached: u64,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupplierBinding {
    provider: String,
    provider_name: String,
    source: String,
    upstream: UpstreamBinding,
    session_binding: SessionBinding,
    competition: Competition,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpstreamBinding {
    supplier: String,
    id: String,
    request_id: String,
    session_id: String,
    timestamp: String,
    model: String,
    input: u64,
    output: u64,
    cached: u64,
}

impl UpstreamBinding {
    fn request_key(&self) -> String {
        serde_json::to_string(&(&self.supplier, &self.request_id)).unwrap_or_default()
    }
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionBinding {
    field: String,
    value: String,
    provenance: String,
    header_population_verified: bool,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Competition {
    primary_window_ms: u64,
    guard_ms: u64,
    native_count: u64,
    extra_local_count: u64,
    upstream_count: u64,
    local_ids: Vec<String>,
    upstream_ids: Vec<String>,
    model_filter: bool,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelBinding {
    status: String,
    recorded_model: String,
    upstream_model: String,
    action: String,
}

fn text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn event_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn session_id(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn supplier_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn field_path(value: &str) -> bool {
    text(value)
        && value.split('.').all(|part| {
            let part = part.strip_suffix("[]").unwrap_or(part);
            !part.is_empty()
                && (part.as_bytes()[0].is_ascii_alphabetic() || part.as_bytes()[0] == b'_')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

impl SupplierRecord {
    fn valid(&self) -> bool {
        let event = &self.event;
        let supplier = &self.supplier_evidence;
        let upstream = &supplier.upstream;
        let binding = &supplier.session_binding;
        let competition = &supplier.competition;
        let model = &self.model_evidence;
        let near = DateTime::parse_from_rfc3339(&event.timestamp)
            .ok()
            .zip(DateTime::parse_from_rfc3339(&upstream.timestamp).ok())
            .is_some_and(|(local, remote)| {
                local.signed_duration_since(remote).abs() <= Days::seconds(30)
            });
        event_id(&event.event_id)
            && session_id(&event.session)
            && [
                &event.timestamp,
                &event.recorded_model,
                &upstream.timestamp,
                &upstream.model,
            ]
            .iter()
            .all(|value| text(value))
            && !unknown_model(&event.recorded_model)
            && !unknown_model(request_base_model(&event.recorded_model))
            && !unknown_model(&upstream.model)
            && !unknown_model(request_base_model(&upstream.model))
            && matches!(event.role.as_str(), "main" | "subagent")
            && event.cached <= event.input
            && event
                .input
                .checked_add(event.output)
                .is_some_and(|sum| sum > 0)
            && text(&supplier.provider)
            && !unknown_provider(&supplier.provider)
            && text(&supplier.provider_name)
            && supplier.source == SOURCE
            && supplier_id(&upstream.supplier)
            && text(&upstream.id)
            && upstream
                .id
                .split_once(':')
                .is_some_and(|(namespace, id)| namespace == upstream.supplier && text(id))
            && text(&upstream.request_id)
            && upstream.session_id == event.session
            && upstream.input == event.input
            && upstream.output == event.output
            && upstream.cached == event.cached
            && near
            && field_path(&binding.field)
            && binding.value == event.session
            && binding.provenance == "direct-sanitized-scalar-copy"
            && !binding.header_population_verified
            && competition.primary_window_ms == 30_000
            && competition.guard_ms == 300_000
            && competition.native_count > 0
            && competition.upstream_count > 0
            && competition.native_count <= 9_007_199_254_740_991
            && competition.extra_local_count <= 9_007_199_254_740_991
            && competition.upstream_count <= 9_007_199_254_740_991
            && competition.local_ids.len() == 1
            && competition.local_ids[0] == event.event_id
            && competition.upstream_ids.len() == 1
            && competition.upstream_ids[0] == upstream.id
            && !competition.model_filter
            && model.status == "conflict"
            && model.action == "preserve-recorded"
            && model.recorded_model == event.recorded_model
            && model.upstream_model == upstream.model
            && model.recorded_model != model.upstream_model
    }

    fn matches(&self, event: &UsageEvent) -> bool {
        let binding = &self.event;
        binding.session == event.session
            && DateTime::parse_from_rfc3339(&binding.timestamp).ok()
                == DateTime::parse_from_rfc3339(&event.timestamp).ok()
            && binding.recorded_model == event.model
            && binding.role == event.role
            && binding.input == event.delta.input_tokens
            && binding.output == event.delta.output_tokens
            && binding.cached == event.delta.cached_input_tokens
            && binding.input.checked_add(binding.output) == Some(event.delta.total_tokens)
    }
}

fn observed_file_identity(path: &Path, metadata: &fs::Metadata) -> io::Result<(u64, u64)> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let _ = metadata;
        let file = fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(0x0020_0000)
            .open(path)?;
        codex_plus_core::codex_sqlite::opened_file_identity(&file)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        file_identity(metadata)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "file identity unavailable"))
    }
}

fn read_document(settings: &Path) -> anyhow::Result<Option<Document>> {
    let path = settings.with_file_name("usage-supplier-evidence-v2.json");
    let metadata = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    };
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "evidence size/type"
    );
    let identity = observed_file_identity(&path, &metadata)?;
    let stamp = |value: &fs::Metadata| (value.len(), value.modified().ok());
    let mut file = File::open(&path)?;
    let opened = file.metadata()?;
    anyhow::ensure!(
        stamp(&opened) == stamp(&metadata)
            && codex_plus_core::codex_sqlite::opened_file_identity(&file)? == identity,
        "evidence replaced"
    );
    let mut bytes = Vec::new();
    (&mut file).take(LIMIT + 1).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(&path)?;
    let opened_after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == metadata.len()
            && after.is_file()
            && stamp(&after) == stamp(&metadata)
            && stamp(&opened_after) == stamp(&metadata)
            && observed_file_identity(&path, &after)? == identity
            && codex_plus_core::codex_sqlite::opened_file_identity(&file)? == identity,
        "evidence changed"
    );
    let document: Document = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        document.version == 2
            && !document.records.is_empty()
            && document.records.len() <= MAX_EVENTS,
        "evidence version/count"
    );
    Ok(Some(document))
}

fn claim(owners: &mut HashMap<String, Option<String>>, id: &str, owner: &str) -> bool {
    if let Some(previous) = owners.get_mut(id) {
        if previous.as_deref() != Some(owner) {
            *previous = None;
        }
        return previous.as_deref() == Some(owner);
    }
    if owners.len() >= MAX_EVENTS {
        return false;
    }
    owners.insert(id.to_owned(), Some(owner.to_owned()));
    true
}

impl Cache {
    pub(super) fn supplement(&mut self, settings: &Path, snapshot: &mut Snapshot) {
        let result = self.load(settings, snapshot);
        let mut report = result.unwrap_or_else(|_| json!({"status":"rejected"}));
        report["source"] = json!(SOURCE);
        report["trustBoundary"] = json!("external-offline-record-bound");
        report["runtimeCompetitionRecomputed"] = json!(false);
        report["modelRepairs"] = json!(0);
        snapshot.supplier_evidence = report;
    }

    fn load(&mut self, settings: &Path, snapshot: &mut Snapshot) -> anyhow::Result<Value> {
        let Some(document) = read_document(settings)? else {
            return Ok(json!({"status":"absent"}));
        };
        let mut records = HashMap::new();
        let mut upstream_counts = HashMap::new();
        let mut request_counts = HashMap::new();
        let mut invalid = 0usize;
        for record in document.records {
            let id = &record.event.event_id;
            if !event_id(id)
                || (!self.signatures.contains_key(id) && self.signatures.len() >= MAX_EVENTS)
            {
                invalid += 1;
                continue;
            }
            *upstream_counts
                .entry(record.supplier_evidence.upstream.id.clone())
                .or_insert(0usize) += 1;
            *request_counts
                .entry(record.supplier_evidence.upstream.request_key())
                .or_insert(0usize) += 1;
            let signature: [u8; 32] = Sha256::digest(serde_json::to_vec(&record)?).into();
            let valid = record.valid()
                && !records.contains_key(id)
                && self
                    .signatures
                    .get(id)
                    .is_none_or(|previous| *previous == Some(signature));
            self.signatures
                .insert(id.clone(), valid.then_some(signature));
            if valid {
                claim(
                    &mut self.upstream_owners,
                    &record.supplier_evidence.upstream.id,
                    id,
                );
                claim(
                    &mut self.request_owners,
                    &record.supplier_evidence.upstream.request_key(),
                    id,
                );
            }
            invalid += usize::from(!valid);
            records.insert(id.clone(), valid.then_some(record));
        }
        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        for event in &snapshot.events {
            if !seen.insert(event.id) {
                duplicates.insert(event.id);
            }
        }
        let (mut applied, mut unchanged, mut conflicts, mut mismatches) =
            (0usize, 0usize, 0usize, 0usize);
        let mut model_conflicts = 0usize;
        let mut model_details = Vec::new();
        let mut matched = HashSet::new();
        for event in &mut snapshot.events {
            let id: String = event.id.iter().map(|byte| format!("{byte:02x}")).collect();
            let Some(Some(record)) = records.get(&id) else {
                continue;
            };
            let upstream = &record.supplier_evidence.upstream;
            if upstream_counts.get(&upstream.id) != Some(&1)
                || request_counts.get(&upstream.request_key()) != Some(&1)
                || self
                    .upstream_owners
                    .get(&upstream.id)
                    .and_then(Option::as_deref)
                    != Some(id.as_str())
                || self
                    .request_owners
                    .get(&upstream.request_key())
                    .and_then(Option::as_deref)
                    != Some(id.as_str())
                || duplicates.contains(&event.id)
                || event.context_conflicts.any()
                || event.count_uncertain
                || event.count_incomplete
                || !event.delta.cached_known
                || !event.total.cached_known
            {
                conflicts += 1;
                continue;
            }
            if !record.matches(event) {
                mismatches += 1;
                continue;
            }
            matched.insert(id.clone());
            let supplier = &record.supplier_evidence;
            if !unknown_provider(&event.provider) && event.provider != supplier.provider {
                conflicts += 1;
                continue;
            }
            if unknown_provider(&event.provider) {
                event.provider.clone_from(&supplier.provider);
                event.provider_source = Some(SOURCE.to_owned());
                applied += 1;
            } else {
                unchanged += 1;
            }
            let name = snapshot
                .provider_names
                .entry(supplier.provider.clone())
                .or_default();
            if name.trim().is_empty() {
                name.clone_from(&supplier.provider_name);
            }
            model_conflicts += 1;
            if model_details.len() < MAX_MODEL_DETAILS {
                model_details.push(json!({
                    "eventId":id,"status":record.model_evidence.status,
                    "recordedModel":record.model_evidence.recorded_model,
                    "upstreamModel":record.model_evidence.upstream_model,
                    "action":record.model_evidence.action
                }));
            }
        }
        Ok(json!({
            "status":"ok","appliedEvents":applied,"unchangedEvents":unchanged,
            "invalid":invalid,"conflictedEvents":conflicts,"signatureMismatches":mismatches,
            "unmatchedRecords":records.values().flatten().filter(|record| !matched.contains(&record.event.event_id)).count(),
            "modelConflictEvents":model_conflicts,"modelEvidence":model_details,
            "modelEvidenceTruncated":model_conflicts > MAX_MODEL_DETAILS
        }))
    }
}

#[cfg(test)]
mod tests;
