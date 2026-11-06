import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MiningEvent, Snapshot } from "./types";
export const desktop = isTauri();
export function command<T>(
  name: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!desktop)
    return Promise.reject(
      new Error(
        "Open the installed PengMiner desktop app to connect to the mining backend.",
      ),
    );
  return invoke<T>(name, args);
}
export async function subscribe(onEvent: (event: MiningEvent) => void) {
  if (!desktop) return () => {};
  return listen<MiningEvent>("pengminer:event", (event) =>
    onEvent(event.payload),
  );
}
export interface Sample {
  time: number;
  rate: number;
}
export function appendSample(samples: Sample[], event: MiningEvent): Sample[] {
  if (event.kind === "mining_started") return [];
  if (event.kind !== "hashrate_updated") return samples;
  return [
    ...samples,
    { time: event.snapshot.uptime, rate: event.snapshot.currentHashrate },
  ].slice(-90);
}
export function elapsed(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  return [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60]
    .map((part) => part.toString().padStart(2, "0"))
    .join(":");
}
export function hashrate(value: number) {
  return value >= 1e6
    ? `${(value / 1e6).toFixed(2)} MH/s`
    : value >= 1e3
      ? `${(value / 1e3).toFixed(2)} kH/s`
      : `${value.toFixed(0)} H/s`;
}
export function pengBalance(state: Snapshot) {
  return (
    state.balances.find((balance) => balance.symbol === "PENG")?.amount ?? 0
  );
}
