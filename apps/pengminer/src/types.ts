export interface Settings {
  server: string;
  token: string;
  network: string;
  device: string;
  cpuThreads: number;
  intensity: number;
  startOnLaunch: boolean;
  minimizeToTray: boolean;
  theme: string;
  notifications: boolean;
}
export interface Gpu {
  id: string;
  index: number;
  name: string;
  vendor: string;
  deviceType: string;
  memoryBytes: number;
  integrated: boolean;
  available: boolean;
}
export interface Job {
  jobId: string;
  tokenId: string;
  walletId: string;
  challenge: string;
  difficulty: number;
  expiresAt: number;
}
export interface Solution {
  time: number;
  walletId: string;
  server: string;
  token: string;
  jobId: string;
  nonce: string;
  accepted: boolean;
  reward: number;
}
export interface Snapshot {
  running: boolean;
  connected: boolean;
  walletId: string | null;
  persistent: boolean;
  settings: Settings;
  currentHashrate: number;
  averageHashrate: number;
  hashes: number;
  accepted: number;
  rejected: number;
  earnings: number;
  difficulty: number | null;
  uptime: number;
  deviceName: string;
  job: Job | null;
  balances: { tokenId: string; symbol: string; amount: number }[];
  tokens: {
    id: string;
    symbol: string;
    name: string;
    mineable: number;
    difficulty: number;
  }[];
  history: Solution[];
  logs: { time: number; kind: string; message: string }[];
  auditLogs: {
    id: number;
    eventType: string;
    payload: string;
    createdAt: number;
    hash: string;
  }[];
  error: string | null;
  cpuCount: number;
}
export interface MiningEvent {
  kind: string;
  snapshot: Snapshot;
}
