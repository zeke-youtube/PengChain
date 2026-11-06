import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MiningEvent, Snapshot } from "./types";
const mock = vi.hoisted(() => ({
  command: vi.fn(),
  subscribe: vi.fn(),
  unlisten: vi.fn(),
}));
vi.mock("./bridge", async (original) => ({
  ...(await original<typeof import("./bridge")>()),
  desktop: true,
  command: mock.command,
  subscribe: mock.subscribe,
}));
import App from "./App";
import { appendSample } from "./bridge";
let callback: (event: MiningEvent) => void;
const snapshot = (): Snapshot => ({
  running: false,
  connected: true,
  walletId: null,
  persistent: false,
  settings: {
    enginePath: "",
    server: "http://127.0.0.1:3000",
    token: "PENG",
    network: "pengchain-v1",
    device: "cpu",
    cpuThreads: 1,
    intensity: 100,
    startOnLaunch: false,
    minimizeToTray: false,
    theme: "dark",
    notifications: false,
  },
  currentHashrate: 0,
  averageHashrate: 0,
  hashes: 0,
  accepted: 0,
  rejected: 0,
  earnings: 0,
  difficulty: null,
  uptime: 0,
  deviceName: "CPU",
  job: null,
  balances: [],
  tokens: [],
  history: [],
  logs: [],
  error: null,
  cpuCount: 4,
});
beforeEach(() => {
  vi.clearAllMocks();
  mock.subscribe.mockImplementation(async (fn) => {
    callback = fn;
    return mock.unlisten;
  });
  mock.command.mockImplementation(async (name: string) =>
    name === "devices" ? { gpus: [], warning: null } : snapshot(),
  );
});
describe("PengMiner desktop UI", () => {
  it("has honest empty charts, no auto-mining, and disabled PPU hardware", async () => {
    render(<App />);
    await screen.findByText("Connect your wallet");
    expect(
      screen.getByText("Start mining to see measured performance"),
    ).toBeInTheDocument();
    expect(
      mock.command.mock.calls.some(([name]) => name === "start_mining"),
    ).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Devices" }));
    expect(
      screen.getByRole("button", { name: "PPU mining unavailable" }),
    ).toBeDisabled();
    expect(screen.getAllByText("Not implemented")).toHaveLength(2);
  });
  it("signs in through IPC and clears the password field", async () => {
    render(<App />);
    await screen.findByText("Connect your wallet");
    fireEvent.change(screen.getByLabelText("Wallet ID"), {
      target: { value: "PENG-TEST" },
    });
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "temporary-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Sign in/ }));
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith("login", {
        wallet: "PENG-TEST",
        password: "temporary-password",
      }),
    );
    expect(screen.getByLabelText("Password")).toHaveValue("");
  });
  it("renders measured backend events, locks configuration, and stops mining", async () => {
    render(<App />);
    await waitFor(() => expect(mock.subscribe).toHaveBeenCalled());
    const live = {
      ...snapshot(),
      running: true,
      walletId: "PENG-TEST",
      currentHashrate: 2345,
      averageHashrate: 1500,
      hashes: 5000,
      earnings: 10,
      accepted: 1,
      difficulty: 3,
    };
    act(() => callback({ kind: "hashrate_updated", snapshot: live }));
    expect(screen.getAllByText("2.35 kH/s").length).toBe(2);
    expect(
      screen.getByRole("combobox", { name: "Compute device" }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "STOP MINING" }));
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith("stop_mining", undefined),
    );
  });
  it("shows network errors and unregisters event listeners", async () => {
    const view = render(<App />);
    await waitFor(() => expect(mock.subscribe).toHaveBeenCalled());
    act(() =>
      callback({
        kind: "network_disconnected",
        snapshot: {
          ...snapshot(),
          connected: false,
          error: "Connection lost.",
        },
      }),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Connection lost.");
    view.unmount();
    expect(mock.unlisten).toHaveBeenCalledOnce();
  });
  it("bounds actual telemetry history and resets on a new mining session", () => {
    const event = {
      kind: "hashrate_updated",
      snapshot: { ...snapshot(), currentHashrate: 55 },
    };
    expect(
      appendSample(Array(90).fill({ time: 0, rate: 1 }), event),
    ).toHaveLength(90);
    expect(appendSample([], event)).toEqual([{ time: 0, rate: 55 }]);
    expect(
      appendSample([{ time: 0, rate: 1 }], {
        ...event,
        kind: "mining_started",
      }),
    ).toEqual([]);
  });
});
