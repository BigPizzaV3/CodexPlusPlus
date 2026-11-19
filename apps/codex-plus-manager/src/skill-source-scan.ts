import type { SkillRepository } from "./skill-repository";

export type SkillSourceMatch = {
  previewId: string; localId: string; localName: string;
  repoKey: string; repoPath: string; remoteName: string; exact: boolean;
  conflict?: string | null; localPaths?: string[];
};
export type SkillSourceCandidate = SkillSourceMatch & { manual: boolean };
export type SkillSourceEvidence = { localId: string; repo: string; kind: string; path: string };
type ScanResult = {
  status: string; message: string; previewId: string; matches: Omit<SkillSourceMatch, "previewId">[];
  manualCandidates?: Omit<SkillSourceMatch, "previewId">[]; warnings: string[];
};
type EvidenceResult = { status: string; message: string; evidence: Omit<SkillSourceEvidence, "localId">[] };
type ScanRequest = (command: string, args: Record<string, unknown>) => Promise<unknown>;
export type SkillSourceScanSnapshot = {
  status: "idle" | "scanning" | "paused" | "complete";
  repos: SkillRepository[];
  currentRepo: string;
  candidates: SkillSourceCandidate[];
  warnings: string[];
  progress: number;
  evidence: SkillSourceEvidence[];
  evidenceErrors: string[];
  evidenceScanning: boolean;
};

export const skillSourceRepoKey = (repo: SkillRepository) =>
  `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`;

export class SkillSourceScan {
  private snapshot: SkillSourceScanSnapshot = {
    status: "idle", repos: [], currentRepo: "", candidates: [], warnings: [], progress: 0,
    evidence: [], evidenceErrors: [], evidenceScanning: false,
  };
  private listeners = new Set<() => void>();
  private generation = 0;
  private running = false;
  private evidenceRunning = false;
  private evidenceIds: string[] = [];
  private evidenceProgress = 0;
  private consumed = new Set<string>();
  private request: ScanRequest;
  private id?: string;
  private localPath?: string;

  constructor(request: ScanRequest, id?: string, localPath?: string) {
    this.request = request;
    this.id = id;
    this.localPath = localPath;
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = () => this.snapshot;
  private publish(update: Partial<SkillSourceScanSnapshot>) {
    this.snapshot = { ...this.snapshot, ...update };
    for (const listener of this.listeners) listener();
  }

  ensure(repos: SkillRepository[], localIds: string[]) {
    if (this.snapshot.status === "idle") {
      this.restart(repos, localIds);
      return;
    }
    for (const repo of repos) this.addRepo(repo);
  }

  restart(repos: SkillRepository[], localIds: string[]) {
    this.generation++;
    this.evidenceIds = [...localIds];
    this.evidenceProgress = 0;
    this.consumed.clear();
    this.publish({
      status: repos.length || localIds.length ? "scanning" : "complete", repos: [...repos], currentRepo: "",
      candidates: [], warnings: [], progress: 0, evidence: [], evidenceErrors: [],
      evidenceScanning: localIds.length > 0,
    });
    this.pumpEvidence();
    this.pump();
  }

  addRepo(repo: SkillRepository) {
    const key = skillSourceRepoKey(repo).toLowerCase();
    if (this.snapshot.repos.some((item) => skillSourceRepoKey(item).toLowerCase() === key)) return;
    this.publish({
      repos: [...this.snapshot.repos, repo],
      status: this.snapshot.status === "complete" ? "scanning" : this.snapshot.status,
    });
    this.pump();
  }

  pause() {
    if (this.snapshot.status === "scanning") this.publish({ status: "paused" });
  }

  resume() {
    if (this.snapshot.status !== "paused") return;
    this.publish({ status: "scanning" });
    this.pumpEvidence();
    this.pump();
  }

  consume(localIds: string[]) {
    for (const id of localIds) this.consumed.add(id);
    this.publish({ candidates: this.snapshot.candidates.filter((candidate) => !this.consumed.has(candidate.localId)) });
  }

  dispose() {
    this.generation++;
    this.publish({ status: "idle" });
  }

  private finishIfComplete() {
    if (this.snapshot.status === "scanning"
      && this.snapshot.progress >= this.snapshot.repos.length
      && this.evidenceProgress >= this.evidenceIds.length) {
      this.publish({ status: "complete" });
    }
  }

  private pumpEvidence() {
    if (this.evidenceRunning || this.snapshot.status !== "scanning") return;
    if (this.evidenceProgress >= this.evidenceIds.length) {
      if (this.snapshot.evidenceScanning) this.publish({ evidenceScanning: false });
      this.finishIfComplete();
      return;
    }
    this.evidenceRunning = true;
    if (!this.snapshot.evidenceScanning) this.publish({ evidenceScanning: true });
    const generation = this.generation;
    void (async () => {
      while (generation === this.generation && this.snapshot.status === "scanning"
        && this.evidenceProgress < this.evidenceIds.length) {
        const localId = this.evidenceIds[this.evidenceProgress];
        try {
          const result = await this.request("discover_skill_source_evidence", {
            id: localId, localPath: this.localPath || null,
          }) as EvidenceResult;
          if (generation !== this.generation) return;
          if (result.status === "ok") this.publish({
            evidence: [...this.snapshot.evidence, ...result.evidence.map((item) => ({ ...item, localId }))],
          });
          else this.publish({ evidenceErrors: [...this.snapshot.evidenceErrors, `${localId}：${result.message}`] });
        } catch (error) {
          if (generation !== this.generation) return;
          this.publish({ evidenceErrors: [...this.snapshot.evidenceErrors, `${localId}：${String(error)}`] });
        }
        this.evidenceProgress++;
      }
      if (generation === this.generation) {
        this.publish({ evidenceScanning: false });
        this.finishIfComplete();
      }
    })().finally(() => {
      this.evidenceRunning = false;
      if (this.snapshot.status === "scanning") this.pumpEvidence();
      else if (this.snapshot.evidenceScanning) this.publish({ evidenceScanning: false });
    });
  }

  private pump() {
    if (this.running || this.snapshot.status !== "scanning") return;
    if (this.snapshot.progress >= this.snapshot.repos.length) {
      this.finishIfComplete();
      return;
    }
    this.running = true;
    const generation = this.generation;
    void (async () => {
      while (generation === this.generation && this.snapshot.status === "scanning"
        && this.snapshot.progress < this.snapshot.repos.length) {
        const repo = this.snapshot.repos[this.snapshot.progress];
        const repoKey = skillSourceRepoKey(repo);
        this.publish({ currentRepo: repoKey });
        try {
          const result = await this.request("find_skill_repository_matches", {
            repoKey, id: this.id || null, localPath: this.localPath || null,
          }) as ScanResult;
          if (generation !== this.generation) return;
          if (result.status !== "ok") this.publish({ warnings: [...this.snapshot.warnings, `${repoKey}：${result.message}`] });
          else {
            const matches = result.matches.map((match) => ({ ...match, previewId: result.previewId, manual: false }));
            const manual = this.id ? (result.manualCandidates || []).filter((match) => match.localId === this.id)
              .map((match) => ({ ...match, previewId: result.previewId, manual: true })) : [];
            this.publish({
              candidates: [...this.snapshot.candidates, ...[...matches, ...manual]
                .filter((candidate) => !this.consumed.has(candidate.localId))],
              warnings: [...this.snapshot.warnings, ...result.warnings],
            });
          }
        } catch (error) {
          if (generation !== this.generation) return;
          this.publish({ warnings: [...this.snapshot.warnings, `${repoKey}：${String(error)}`] });
        }
        this.publish({ currentRepo: "", progress: this.snapshot.progress + 1 });
      }
      if (generation === this.generation) this.finishIfComplete();
    })().finally(() => {
      this.running = false;
      if (this.snapshot.status === "scanning") this.pump();
    });
  }
}

const scans = new Map<string, SkillSourceScan>();
export function getSkillSourceScan(request: ScanRequest, id?: string, localPath?: string) {
  const key = JSON.stringify([id || "", localPath || ""]);
  let scan = scans.get(key);
  if (!scan) {
    scan = new SkillSourceScan(request, id, localPath);
    scans.set(key, scan);
  }
  return scan;
}

export function discardSkillSourceScan(scan: SkillSourceScan, id?: string, localPath?: string) {
  const key = JSON.stringify([id || "", localPath || ""]);
  if (scans.get(key) === scan) {
    scans.delete(key);
    scan.dispose();
  }
}
