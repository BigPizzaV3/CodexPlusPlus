use super::*;

static SERIAL: Mutex<()> = Mutex::new(());

fn buffered_file() -> FileIndex {
    let mut file = FileIndex::new();
    assert!(file.pending.extend(b"{"));
    file
}

#[test]
fn third_home_evicts_the_least_recently_used_of_two_cached_homes() {
    let _serial = SERIAL.lock().unwrap();
    let homes: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let paths: Vec<_> = homes.iter().map(|home| home.path().to_path_buf()).collect();
    let mut cache = HistoryCache::default();
    drop(cache.get(paths[0].clone()));
    let second = cache.get(paths[1].clone());
    let released = Arc::downgrade(&second);
    drop(second);
    cache.homes.get_mut(&paths[1]).unwrap().touched = Instant::now() - Duration::from_secs(2);
    drop(cache.get(paths[0].clone()));
    drop(cache.get(paths[2].clone()));
    assert_eq!(cache.homes.len(), MAX_HOMES);
    assert!(cache.homes.contains_key(&paths[0]));
    assert!(!cache.homes.contains_key(&paths[1]));
    assert!(cache.homes.contains_key(&paths[2]));
    assert!(released.upgrade().is_none());
}

#[test]
fn idle_expiry_keeps_an_active_lease_then_releases_its_pending_memory() {
    let _serial = SERIAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut cache = HistoryCache::default();
    let leased = cache.get(home.path().to_path_buf());
    let released = Arc::downgrade(&leased);
    let before = PENDING_BYTES.load(Ordering::Acquire);
    leased
        .lock()
        .unwrap()
        .files
        .insert(home.path().join("fixture.jsonl"), buffered_file());
    cache.homes.get_mut(home.path()).unwrap().touched =
        Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
    cache.expire();
    assert_eq!(cache.homes.len(), 1);
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), before + MAX_LINE);
    drop(leased);
    cache.expire();
    assert!(cache.homes.is_empty());
    assert!(released.upgrade().is_none());
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), before);
}

#[test]
fn global_half_line_budget_rejects_then_recovers_after_clear_and_drop() {
    let _serial = SERIAL.lock().unwrap();
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), 0);
    let mut lines: Vec<_> = (0..MAX_PENDING_BYTES / MAX_LINE)
        .map(|_| {
            let mut line = PendingLine::default();
            assert!(line.extend(b"{"));
            line
        })
        .collect();
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), MAX_PENDING_BYTES);
    let mut blocked = PendingLine::default();
    assert!(!blocked.extend(b"{"));
    assert_eq!(blocked.len(), 0);
    lines[0].clear();
    assert!(blocked.extend(b"{"));
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), MAX_PENDING_BYTES);
    drop(blocked);
    drop(lines);
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), 0);
}

#[test]
fn idle_reaper_and_explicit_clear_release_cache_without_changing_files() {
    let _serial = SERIAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(home.path()).unwrap();
    let marker = path.join("synthetic-marker.txt");
    fs::write(&marker, b"unchanged synthetic fixture").unwrap();
    let before = PENDING_BYTES.load(Ordering::Acquire);
    let cache = CACHE.get_or_init(|| Mutex::new(HistoryCache::default()));
    let released = {
        let mut cache = cache.lock().unwrap();
        assert!(cache.homes.is_empty());
        let index = cache.get(path.clone());
        index
            .lock()
            .unwrap()
            .files
            .insert(path.join("fixture.jsonl"), buffered_file());
        cache.homes.get_mut(&path).unwrap().touched =
            Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        cache.reaper_running = true;
        Arc::downgrade(&index)
    };
    expire_idle();
    assert!(released.upgrade().is_none());
    assert!(cache.lock().unwrap().homes.is_empty());
    assert!(!cache.lock().unwrap().reaper_running);
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), before);
    {
        let mut cache = cache.lock().unwrap();
        let index = cache.get(path.clone());
        index
            .lock()
            .unwrap()
            .files
            .insert(path.join("fixture.jsonl"), buffered_file());
    }
    clear_history(&path);
    clear_history(&path);
    assert!(cache.lock().unwrap().homes.is_empty());
    assert_eq!(PENDING_BYTES.load(Ordering::Acquire), before);
    assert_eq!(fs::read(marker).unwrap(), b"unchanged synthetic fixture");
}
