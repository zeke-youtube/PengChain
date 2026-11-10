import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MiningEvent, Snapshot } from "./types";
interface AndroidBridge {
  request(id: string, name: string, args: string): void;
}
declare global {
  interface Window {
    PengMinerAndroid?: AndroidBridge;
  }
}
export const android =
  typeof window !== "undefined" && !!window.PengMinerAndroid;
export const desktop = android || isTauri();
let nextRequest = 0;
const pending = new Map<
  string,
  { resolve(value: unknown): void; reject(error: Error): void }
>();
if (android)
  window.addEventListener("pengminer:response", (event) => {
    const { id, result, error } = (
      event as CustomEvent<{ id: string; result: unknown; error?: string }>
    ).detail;
    const request = pending.get(id);
    if (!request) return;
    pending.delete(id);
    if (error) request.reject(new Error(error));
    else request.resolve(result);
  });
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
  if (android)
    return new Promise<T>((resolve, reject) => {
      const id = String(++nextRequest);
      pending.set(id, { resolve: (value) => resolve(value as T), reject });
      window.PengMinerAndroid!.request(id, name, JSON.stringify(args ?? {}));
    });
  return invoke<T>(name, args);
}
export async function subscribe(onEvent: (event: MiningEvent) => void) {
  if (!desktop) return () => {};
  if (android) {
    const listener = (event: Event) =>
      onEvent((event as CustomEvent<MiningEvent>).detail);
    window.addEventListener("pengminer:event", listener);
    return () => window.removeEventListener("pengminer:event", listener);
  }
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
