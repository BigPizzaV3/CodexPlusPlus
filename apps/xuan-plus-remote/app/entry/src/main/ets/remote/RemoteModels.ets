export enum RemoteEnvironment {
  DEV = 'dev',
  PROD = 'prod'
}

export enum PcConnectionState {
  ONLINE = 'online',
  STALE = 'stale',
  OFFLINE = 'offline'
}

export enum PairingState {
  UNBOUND = 'unbound',
  PENDING_LOCAL_CONFIRMATION = 'pending_local_confirmation',
  ACTIVE = 'active',
  FAILED = 'failed'
}

export enum TaskStatus {
  CREATED = 'created',
  QUEUED = 'queued',
  STARTING = 'starting',
  RUNNING = 'running',
  PAUSING = 'pausing',
  PAUSED = 'paused',
  STOPPING = 'stopping',
  STOPPED = 'stopped',
  FAILED = 'failed',
  RECONCILING = 'reconciling',
  ARCHIVED = 'archived'
}

export enum TurnStatus {
  IDLE = 'idle',
  STARTING = 'starting',
  RUNNING = 'running',
  PAUSING = 'pausing',
  COMPLETED = 'completed',
  FAILED = 'failed',
  INTERRUPTED = 'interrupted',
  STOPPED = 'stopped',
  RECONCILING = 'reconciling'
}

export enum ReplyState {
  AVAILABLE = 'available',
  ABSENT = 'absent',
  WITHHELD = 'withheld',
  TRUNCATED = 'truncated'
}

export enum CommandType {
  CREATE_TASK = 'create_task',
  START_TASK = 'start_task',
  SEND_INPUT = 'send_input',
  PAUSE_TASK = 'pause_task',
  RESUME_TASK = 'resume_task',
  STOP_TASK = 'stop_task'
}

export enum CommandStatus {
  QUEUED = 'queued',
  DISPATCHED = 'dispatched',
  ACCEPTED = 'accepted',
  APPLYING = 'applying',
  COMPLETED = 'completed',
  REJECTED = 'rejected',
  FAILED = 'failed',
  EXPIRED = 'expired',
  CANCELLED = 'cancelled',
  RECONCILING = 'reconciling'
}

export function utf8ByteLength(value: string): number {
  let bytes: number = 0;
  for (let index: number = 0; index < value.length; index++) {
    const codeUnit: number = value.charCodeAt(index);
    if (codeUnit < 0x80) {
      bytes += 1;
    } else if (codeUnit < 0x800) {
      bytes += 2;
    } else if (codeUnit >= 0xD800 && codeUnit <= 0xDBFF && index + 1 < value.length) {
      const nextCodeUnit: number = value.charCodeAt(index + 1);
      if (nextCodeUnit >= 0xDC00 && nextCodeUnit <= 0xDFFF) {
        bytes += 4;
        index++;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

export class RemoteReply {
  state: ReplyState;
  text: string | null;
  byteLength: number;
  truncated: boolean;

  constructor(state: ReplyState, text: string | null, byteLength: number, truncated: boolean) {
    this.state = state;
    this.text = text;
    this.byteLength = byteLength;
    this.truncated = truncated;
  }
}

export class RemoteTaskSummary {
  remoteTaskId: string;
  name: string;
  workspaceName: string;
  taskStatus: TaskStatus;
  turnStatus: TurnStatus;
  stateVersion: number;
  pcConnectionState: PcConnectionState;
  lastReply: RemoteReply | null;

  constructor(remoteTaskId: string, name: string, workspaceName: string, taskStatus: TaskStatus,
    turnStatus: TurnStatus, stateVersion: number, pcConnectionState: PcConnectionState,
    lastReply: RemoteReply | null) {
    this.remoteTaskId = remoteTaskId;
    this.name = name;
    this.workspaceName = workspaceName;
    this.taskStatus = taskStatus;
    this.turnStatus = turnStatus;
    this.stateVersion = stateVersion;
    this.pcConnectionState = pcConnectionState;
    this.lastReply = lastReply;
  }
}
