import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import {
  Activity,
  ArrowDownLeft,
  ArrowUpRight,
  Check,
  ChevronRight,
  CircleHelp,
  Cpu,
  Gauge,
  LayoutDashboard,
  LogOut,
  Pause,
  Play,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Sparkles,
  Terminal,
  Wallet,
  Wifi,
  WifiOff,
  Zap,
} from "lucide-react";
import {
  appendSample,
  command,
  desktop,
  elapsed,
  hashrate,
  pengBalance,
  subscribe,
  type Sample,
} from "./bridge";
import type { Gpu, MiningEvent, Settings, Snapshot } from "./types";

const defaults: Settings = {
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
};
const pages = [
  ["Dashboard", LayoutDashboard],
  ["Devices", Cpu],
  ["Mining", Gauge],
  ["Wallet", Wallet],
  ["Settings", Settings2],
  ["Logs", Terminal],
] as const;
const number = (value: number | undefined) =>
  value === undefined ? "—" : value.toLocaleString();
const time = (value: number) => new Date(value).toLocaleTimeString();

function Chart({ samples }: { samples: Sample[] }) {
  const max = Math.max(1, ...samples.map((sample) => sample.rate));
  const first = samples[0]?.time ?? 0;
  const span = (samples.at(-1)?.time ?? first) - first;
  const points = samples
    .map(
      (sample) =>
        `${((sample.time - first) / Math.max(0.001, span)) * 900},${160 - (sample.rate / max) * 140}`,
    )
    .join(" ");
  return (
    <div className="chart" aria-label="Live measured hashrate chart">
      <div className="chart-grid" />
      {samples.length > 1 ? (
        <svg
          viewBox="0 0 900 180"
          preserveAspectRatio="none"
          role="img"
          aria-label="PCA-10 evaluations per second"
        >
          <defs>
            <linearGradient id="fill" x1="0" x2="0" y1="0" y2="1">
              <stop offset="0%" stopColor="#45e7a0" stopOpacity=".25" />
              <stop offset="100%" stopColor="#45e7a0" stopOpacity="0" />
            </linearGradient>
          </defs>
          <polygon points={`0,180 ${points} 900,180`} fill="url(#fill)" />
          <polyline
            points={points}
            fill="none"
            stroke="#45e7a0"
            strokeWidth="2.5"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      ) : (
        <div className="chart-empty">
          <Activity size={25} />
          <span>Start mining to see measured performance</span>
        </div>
      )}
      <div className="chart-axis">
        <span>
          {samples.length > 1
            ? `${span.toFixed(1)} seconds ago`
            : "No samples yet"}
        </span>
        <span>Live · 500 ms samples</span>
        <span>Now</span>
      </div>
    </div>
  );
}
function Stat({
  label,
  value,
  detail,
  accent = false,
}: {
  label: string;
  value: string;
  detail: string;
  accent?: boolean;
}) {
  return (
    <div className={`stat panel ${accent ? "accent-stat" : ""}`}>
      <span className="eyebrow">{label}</span>
      <strong>{value}</strong>
      <span className="muted small">{detail}</span>
    </div>
  );
}
function History({ state }: { state: Snapshot | null }) {
  const rows = [...(state?.history ?? [])].reverse().slice(0, 20);
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>TIME</th>
            <th>TOKEN / JOB</th>
            <th>STATUS</th>
            <th className="align-right">REWARD</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={`${row.jobId}-${i}`}>
              <td className="muted">{time(row.time)}</td>
              <td>
                <strong>{row.token}</strong>
                <span className="subtext mono">{row.jobId}</span>
              </td>
              <td>
                <span className={`pill ${row.accepted ? "green" : "red"}`}>
                  {row.accepted ? <Check size={12} /> : <Pause size={12} />}{" "}
                  {row.accepted ? "Accepted" : "Rejected"}
                </span>
              </td>
              <td className="align-right reward">
                {row.accepted ? `+${number(row.reward)}` : "0"}{" "}
                <span>{row.token}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && (
        <div className="empty">
          No submitted solutions yet. Verified results will appear here.
        </div>
      )}
    </div>
  );
}

export default function App() {
  const [page, setPage] = useState("Dashboard");
  const [state, setState] = useState<Snapshot | null>(null);
  const [draft, setDraft] = useState(defaults);
  const [gpus, setGpus] = useState<Gpu[]>([]);
  const [gpuWarning, setGpuWarning] = useState<string | null>(null);
  const [samples, setSamples] = useState<Sample[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wallet, setWallet] = useState("");
  const [password, setPassword] = useState("");
  const [tokenForm, setTokenForm] = useState({
    name: "",
    symbol: "",
    maxSupply: "",
    mineable: true,
    miningReward: "1",
    difficulty: "3",
    initialIssuance: "0",
  });
  const savedKey = useRef("");
  const apply = useCallback((snapshot: Snapshot) => {
    setState(snapshot);
    const key = JSON.stringify(snapshot.settings);
    if (savedKey.current !== key) {
      savedKey.current = key;
      setDraft(snapshot.settings);
    }
  }, []);
  useEffect(() => {
    let disposed = false;
    let cleanup = () => {};
    const receive = (event: MiningEvent) => {
      if (!disposed) {
        apply(event.snapshot);
        setSamples((previous) => appendSample(previous, event));
      }
    };
    void subscribe(receive)
      .then((unlisten) => {
        if (disposed) unlisten();
        else cleanup = unlisten;
      })
      .catch(() => {
        if (!disposed)
          setError("Could not subscribe to mining events. Restart PengMiner.");
      });
    if (desktop) {
      void command<Snapshot>("snapshot").then((s) => {
        if (!disposed) apply(s);
      });
      void command<{ gpus: Gpu[]; warning: string | null }>("devices")
        .then((data) => {
          if (!disposed) {
            setGpus(data.gpus);
            setGpuWarning(data.warning);
          }
        })
        .catch(() => {
          if (!disposed)
            setGpuWarning("Device discovery failed. Check the OpenCL driver.");
        });
    }
    const poll = setInterval(() => {
      if (desktop)
        void command<Snapshot>("snapshot")
          .then((s) => {
            if (!disposed) apply(s);
          })
          .catch(() => {});
    }, 3000);
    const network = setInterval(() => {
      if (desktop)
        void command<Snapshot>("refresh")
          .then((s) => {
            if (!disposed) apply(s);
          })
          .catch(() => {});
    }, 15000);
    return () => {
      disposed = true;
      cleanup();
      clearInterval(poll);
      clearInterval(network);
    };
  }, [apply]);
  const act = async (name: string, args?: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    try {
      apply(await command<Snapshot>(name, args));
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  };
  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setDraft((previous) => ({ ...previous, [key]: value }));
  const signIn = (event: FormEvent) => {
    event.preventDefault();
    const secret = password;
    setPassword("");
    void act("login", { wallet, password: secret });
  };
  const theme =
    draft.theme === "system"
      ? window.matchMedia?.("(prefers-color-scheme: light)").matches
        ? "light"
        : "dark"
      : draft.theme;
  const running = state?.running ?? false;
  const disabled = busy || !desktop;
  const count = state?.cpuCount ?? 1;
  const selector = (
    <>
      <label>
        Mining token
        <select
          value={draft.token}
          disabled={running || disabled}
          onChange={(e) => update("token", e.target.value)}
        >
          {!state?.tokens.some((t) => t.symbol === draft.token) && (
            <option>{draft.token}</option>
          )}
          {state?.tokens
            .filter((t) => t.mineable === 1)
            .map((t) => (
              <option key={t.id} value={t.symbol}>
                {t.symbol} · {t.name}
              </option>
            ))}
        </select>
      </label>
      <label>
        Compute device
        <select
          value={draft.device}
          disabled={running || disabled}
          onChange={(e) => update("device", e.target.value)}
        >
          <option value="cpu">CPU · {count} available threads</option>
          <option value="gpu:auto">GPU · prefer a dedicated device</option>
          {gpus.map((g) => (
            <option key={g.id} value={g.id} disabled={!g.available}>
              {g.name}
              {g.integrated ? " · integrated" : ""}
            </option>
          ))}
        </select>
      </label>
      <div className="form-split">
        <label>
          CPU threads
          <input
            type="number"
            min={1}
            max={count}
            value={draft.cpuThreads}
            disabled={running || disabled || draft.device !== "cpu"}
            onChange={(e) => update("cpuThreads", Number(e.target.value))}
          />
        </label>
        <label>
          Intensity
          <select
            value={draft.intensity}
            disabled={running || disabled}
            onChange={(e) => update("intensity", Number(e.target.value))}
          >
            <option value={25}>Low · 25%</option>
            <option value={50}>Balanced · 50%</option>
            <option value={100}>Full · 100%</option>
          </select>
        </label>
      </div>
    </>
  );
  const saveAndStart = async () => {
    setBusy(true);
    setError(null);
    try {
      apply(await command<Snapshot>("save_settings", { settings: draft }));
      apply(await command<Snapshot>("start_mining"));
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  };
  const createToken = async (event: FormEvent) => {
    event.preventDefault();
    await act("create_token", {
      request: {
        name: tokenForm.name,
        symbol: tokenForm.symbol.toUpperCase(),
        maxSupply: tokenForm.maxSupply ? Number(tokenForm.maxSupply) : null,
        mineable: tokenForm.mineable,
        miningReward: tokenForm.mineable ? Number(tokenForm.miningReward) : undefined,
        difficulty: tokenForm.mineable ? Number(tokenForm.difficulty) : undefined,
        initialIssuance: tokenForm.mineable ? 0 : Number(tokenForm.initialIssuance),
      },
    });
    setTokenForm((current) => ({ ...current, name: "", symbol: "" }));
  };
  return (
    <div className="app" data-theme={theme}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-icon">
            <Zap fill="currentColor" size={23} />
          </div>
          <div>
            PengMiner<span>BY PIKASTUDIO</span>
          </div>
          <span className="version">V1</span>
        </div>
        <div className="nav-label">WORKSPACE</div>
        <nav>
          {pages.map(([name, Icon]) => (
            <button
              key={name}
              className={page === name ? "active" : ""}
              onClick={() => setPage(name)}
            >
              <Icon size={18} />
              {name}
              {name === "Mining" && running && <span className="live-dot" />}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="network-mini">
            <span
              className={`status-dot ${state?.connected ? "online" : ""}`}
            />
            <div>
              PengChain
              <span>
                {state?.connected ? "Server connected" : "Server disconnected"}
              </span>
            </div>
            <ShieldCheck size={18} />
          </div>
          <div className="sidebar-foot">
            PikaStudio <span>PengChain V1</span>
          </div>
        </div>
      </aside>
      <main>
        <header className="topbar">
          <div className="breadcrumbs">
            Workspace <ChevronRight size={13} />
            <strong>{page}</strong>
          </div>
          <div className="top-actions">
            <span
              className={`connection ${state?.connected ? "connected" : ""}`}
            >
              {state?.connected ? <Wifi size={14} /> : <WifiOff size={14} />}{" "}
              {state?.connected ? "Connected" : "Disconnected"}
            </span>
            <button
              className="icon-button"
              title="Refresh network and balances"
              aria-label="Refresh network and balances"
              disabled={disabled}
              onClick={() => void act("refresh")}
            >
              <RefreshCw size={16} />
            </button>
            <div className="avatar">P</div>
          </div>
        </header>
        <div className="content">
          <div className="page-heading">
            <div>
              <div className="eyebrow emerald">
                PENGCHAIN / MINING WORKSPACE
              </div>
              <h1>
                {page === "Dashboard" ? "Make your compute count." : page}
              </h1>
              <p>
                {page === "Dashboard"
                  ? "Your hardware. Real proof of work. Verified rewards."
                  : page === "Devices"
                    ? "Choose the hardware that does the work."
                    : page === "Wallet"
                      ? "Your server-managed wallet, all in one place."
                      : page === "Mining"
                        ? "Fine-tune your miner. Stay in control."
                        : page === "Logs"
                          ? "Live diagnostics without session secrets."
                          : "A workspace configured for you."}
              </p>
            </div>
            <span className={`pill ${running ? "green" : "neutral"}`}>
              <span className={`status-dot ${running ? "online" : ""}`} />
              {running ? "Mining active" : "Miner idle"}
            </span>
          </div>
          {!desktop && (
            <div className="notice">
              <CircleHelp size={18} />
              <span>
                Desktop backend unavailable in a browser. Open the Tauri app to
                sign in, discover devices, and mine. No mining data is
                simulated.
              </span>
            </div>
          )}
          {(error || state?.error) && (
            <div className="notice error" role="alert">
              {error || state?.error}
            </div>
          )}
          {!state?.walletId && page !== "Settings" && (
            <section className="panel login-panel">
              <div>
                <div className="section-title">
                  <Wallet size={19} />
                  <h2>Connect your wallet</h2>
                </div>
                <p>Use your existing PengChain wallet to start earning.</p>
                <span className="small muted">
                  Sessions use your OS credential store when available.
                  Passwords are never saved.
                </span>
              </div>
              <form onSubmit={signIn}>
                <label className="sr-only" htmlFor="wallet">
                  Wallet ID
                </label>
                <input
                  id="wallet"
                  placeholder="Wallet ID · PENG-…"
                  value={wallet}
                  onChange={(e) => setWallet(e.target.value)}
                  required
                  disabled={disabled}
                />
                <label className="sr-only" htmlFor="password">
                  Password
                </label>
                <input
                  id="password"
                  type="password"
                  placeholder="Password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  required
                  disabled={disabled}
                />
                <button
                  className="button primary"
                  disabled={disabled || !wallet || !password}
                >
                  Sign in <ArrowUpRight size={15} />
                </button>
              </form>
            </section>
          )}
          {page === "Dashboard" && (
            <>
              <div className="stats-grid">
                <Stat
                  label="CURRENT HASHRATE"
                  value={state ? hashrate(state.currentHashrate) : "—"}
                  detail={
                    state
                      ? `Average ${hashrate(state.averageHashrate)}`
                      : "Waiting for backend"
                  }
                />
                <Stat
                  label="SESSION EARNINGS"
                  value={
                    state
                      ? `${number(state.earnings)} ${state.settings.token}`
                      : "—"
                  }
                  detail="Verified integer base units"
                  accent
                />
                <Stat
                  label="ACCEPTED REWARDS"
                  value={number(state?.accepted)}
                  detail={`${number(state?.rejected)} rejected submissions`}
                />
                <Stat
                  label="MINING UPTIME"
                  value={state ? elapsed(state.uptime) : "—"}
                  detail={
                    running ? "This mining session" : "Ready when you are"
                  }
                />
              </div>
              <div className="dashboard-grid">
                <section className="panel performance">
                  <div className="panel-head">
                    <div>
                      <h2>Hashrate performance</h2>
                      <p>Completed PCA-10 evaluations per second</p>
                    </div>
                    <span className="live-label">
                      <span
                        className={`status-dot ${running ? "online" : ""}`}
                      />
                      LIVE
                    </span>
                  </div>
                  <div className="chart-value">
                    {state ? hashrate(state.currentHashrate) : "—"}
                    <span>current output</span>
                  </div>
                  <Chart samples={samples} />
                  <div className="performance-foot">
                    <span>Device: {state?.deviceName ?? "—"}</span>
                    <span>
                      <Activity size={14} /> {number(state?.hashes)} hashes
                      computed
                    </span>
                    <span>
                      Difficulty <strong>{state?.difficulty ?? "—"}</strong>
                    </span>
                  </div>
                </section>
                <section className="panel controls">
                  <div className="section-title">
                    <Zap size={18} />
                    <h2>Mining control</h2>
                  </div>
                  {selector}
                  <button
                    className={`button wide ${running ? "stop" : "primary"}`}
                    disabled={disabled || (!running && !state?.walletId)}
                    onClick={() =>
                      running ? void act("stop_mining") : void saveAndStart()
                    }
                  >
                    {running ? (
                      <Pause size={17} />
                    ) : (
                      <Play size={17} fill="currentColor" />
                    )}
                    {busy
                      ? "Working…"
                      : running
                        ? "STOP MINING"
                        : "START MINING"}
                  </button>
                  <p className="control-note">
                    <ShieldCheck size={12} /> Rewards are verified by the
                    PengChain server.
                  </p>
                </section>
              </div>
              <div className="bottom-grid">
                <section className="panel">
                  <div className="panel-head">
                    <div>
                      <h2>Recent solutions</h2>
                      <p>Server-acknowledged results from your wallet</p>
                    </div>
                    <button
                      className="text-button"
                      onClick={() => setPage("Wallet")}
                    >
                      View wallet <ArrowUpRight size={14} />
                    </button>
                  </div>
                  <History state={state} />
                </section>
                <section className="panel wallet-summary">
                  <div className="eyebrow">YOUR PENG BALANCE</div>
                  <Wallet size={25} />
                  <strong>
                    {state?.walletId ? number(pengBalance(state)) : "—"}{" "}
                    <span>PENG</span>
                  </strong>
                  <p>Available base units</p>
                  <div className="wallet-id">
                    {state?.walletId ?? "No wallet connected"}
                  </div>
                  <button
                    className="button secondary wide"
                    onClick={() => setPage("Wallet")}
                  >
                    Open wallet <ArrowUpRight size={15} />
                  </button>
                </section>
              </div>
            </>
          )}
          {page === "Devices" && (
            <>
              <div className="device-grid">
                <section className="panel device-card">
                  <div className="device-art">
                    <Cpu size={38} />
                    <span className="pill green">Available</span>
                  </div>
                  <h2>System CPU</h2>
                  <p>
                    {state
                      ? `${count} logical threads available`
                      : "Waiting for desktop hardware detection"}
                  </p>
                  <dl>
                    <dt>Identifier</dt>
                    <dd>cpu</dd>
                    <dt>Backend</dt>
                    <dd>Native Rust PCA-10</dd>
                  </dl>
                  <button
                    className="button secondary"
                    disabled={running || disabled}
                    onClick={() => update("device", "cpu")}
                  >
                    {draft.device === "cpu" ? "Selected" : "Select CPU"}
                  </button>
                </section>
                {gpus.map((g) => (
                  <section className="panel device-card" key={g.id}>
                    <div className="device-art">
                      <Gauge size={38} />
                      <span
                        className={`pill ${g.available ? "green" : "neutral"}`}
                      >
                        {g.available ? "Available" : "Unavailable"}
                      </span>
                    </div>
                    <h2>{g.name}</h2>
                    <p>
                      {g.vendor} ·{" "}
                      {g.integrated
                        ? "Integrated"
                        : "Dedicated / discrete reported"}
                    </p>
                    <dl>
                      <dt>Global memory</dt>
                      <dd>{(g.memoryBytes / 2 ** 30).toFixed(1)} GiB</dd>
                      <dt>Device type</dt>
                      <dd>{g.deviceType} / OpenCL</dd>
                      <dt>Identifier</dt>
                      <dd>{g.id}</dd>
                    </dl>
                    <button
                      className="button secondary"
                      disabled={running || disabled || !g.available}
                      onClick={() => update("device", g.id)}
                    >
                      {draft.device === g.id ? "Selected" : "Select GPU"}
                    </button>
                  </section>
                ))}
                <section className="panel device-card ppu">
                  <div className="device-art">
                    <Sparkles size={38} />
                    <span className="pill neutral">Not implemented</span>
                  </div>
                  <h2>Peng Processing Unit</h2>
                  <p>Purpose-built compute. A future chapter.</p>
                  <dl>
                    <dt>Connection</dt>
                    <dd>Planned USB</dd>
                    <dt>Detection</dt>
                    <dd>Not implemented</dd>
                  </dl>
                  <button className="button secondary" disabled>
                    PPU mining unavailable
                  </button>
                </section>
              </div>
              {gpuWarning && <div className="notice">{gpuWarning}</div>}
              <div className="notice">
                <ShieldCheck size={18} />
                OpenCL device metadata is reported by the driver. GPU mode never
                falls back to CPU. Device choices are applied when you save
                settings or start mining.
              </div>
              <button
                className="button primary"
                disabled={disabled || running}
                onClick={() => void act("save_settings", { settings: draft })}
              >
                Save device preference <Check size={16} />
              </button>
            </>
          )}
          {page === "Mining" && (
            <div className="two-col">
              <section className="panel form-panel">
                <h2>Mining configuration</h2>
                <p>Changes apply to the next mining session.</p>
                {selector}
                <label>
                  Network ID
                  <input
                    value={draft.network}
                    disabled={disabled || running}
                    onChange={(e) => update("network", e.target.value)}
                  />
                </label>
                <p className="small muted">
                  CPU intensity controls duty cycle. GPU intensity adjusts batch
                  size and pauses between batches. The mining protocol is
                  unchanged.
                </p>
                <button
                  className={`button wide ${running ? "stop" : "primary"}`}
                  disabled={disabled || (!running && !state?.walletId)}
                  onClick={() =>
                    running ? void act("stop_mining") : void saveAndStart()
                  }
                >
                  {running ? <Pause size={16} /> : <Play size={16} />}{" "}
                  {running ? "STOP MINING" : "START MINING"}
                </button>
              </section>
              <section className="panel form-panel">
                <h2>Active job</h2>
                <p>Fresh server challenges. One reward per job.</p>
                <dl>
                  <dt>Status</dt>
                  <dd>{running ? "Mining" : "Idle"}</dd>
                  <dt>Device</dt>
                  <dd>{state?.deviceName ?? "—"}</dd>
                  <dt>Token</dt>
                  <dd>{state?.settings.token ?? "—"}</dd>
                  <dt>Job ID</dt>
                  <dd className="mono">
                    {state?.job?.jobId ?? "No active job"}
                  </dd>
                  <dt>Difficulty</dt>
                  <dd>{state?.difficulty ?? "—"}</dd>
                  <dt>Expires</dt>
                  <dd>
                    {state?.job
                      ? new Date(state.job.expiresAt).toLocaleTimeString()
                      : "—"}
                  </dd>
                  <dt>Session rewards</dt>
                  <dd>{number(state?.accepted)}</dd>
                </dl>
                <div className="notice small">
                  Stop cancels CPU work immediately and GPU work after the
                  current driver dispatch. Unacknowledged submissions are never
                  retried automatically.
                </div>
              </section>
              <section className="panel span-two">
                <div className="panel-head">
                  <h2>Recent solutions</h2>
                </div>
                <History state={state} />
              </section>
            </div>
          )}
          {page === "Wallet" && (
            <>
              <section className="panel wallet-banner">
                <div>
                  <div className="eyebrow">CONNECTED WALLET</div>
                  <h2 className="mono">{state?.walletId ?? "Not signed in"}</h2>
                  <p>
                    <ShieldCheck size={14} />{" "}
                    {state?.persistent
                      ? "Session protected by OS credential storage"
                      : "Session in memory only; sign in after restarting"}
                  </p>
                </div>
                <button
                  className="button secondary"
                  disabled={disabled || !state?.walletId}
                  onClick={() => void act("logout")}
                >
                  <LogOut size={15} /> Sign out
                </button>
              </section>
              <div className="balance-grid">
                {state?.balances.map((balance) => (
                  <section className="panel balance-card" key={balance.tokenId}>
                    <span className="token-icon">
                      {balance.symbol.slice(0, 1)}
                    </span>
                    <div>
                      <span className="eyebrow">{balance.symbol}</span>
                      <strong>{number(balance.amount)}</strong>
                      <p>Integer base units</p>
                    </div>
                  </section>
                ))}
                {state?.walletId && state.balances.length === 0 && (
                  <div className="panel empty">
                    No balances yet. Mine a token to earn your first verified
                    reward.
                  </div>
                )}
              </div>
              <section className="panel">
                <div className="panel-head">
                  <div>
                    <h2>Mining reward history</h2>
                    <p>
                      Locally retained server receipts for this wallet and
                      server · latest 200
                    </p>
                  </div>
                  <ArrowDownLeft size={19} />
                </div>
                <History state={state} />
              </section>
              <section className="panel form-panel">
                <div className="panel-head">
                  <div>
                    <h2>Create a token</h2>
                    <p>Issue a server-authoritative custom asset from this wallet.</p>
                  </div>
                  <Sparkles size={19} />
                </div>
                <form className="two-col" onSubmit={(event) => void createToken(event)}>
                  <label>Token name<input required value={tokenForm.name} onChange={(event) => setTokenForm({ ...tokenForm, name: event.target.value })} placeholder="Fish Coin" /></label>
                  <label>Symbol<input required maxLength={16} value={tokenForm.symbol} onChange={(event) => setTokenForm({ ...tokenForm, symbol: event.target.value })} placeholder="FISH" /></label>
                  <label>Maximum supply<input type="number" min="1" value={tokenForm.maxSupply} onChange={(event) => setTokenForm({ ...tokenForm, maxSupply: event.target.value })} placeholder="Unlimited" /></label>
                  <label>Mode<select value={tokenForm.mineable ? "mineable" : "issued"} onChange={(event) => setTokenForm({ ...tokenForm, mineable: event.target.value === "mineable" })}><option value="mineable">Mineable</option><option value="issued">Initial issuance</option></select></label>
                  {tokenForm.mineable ? <label>Mining reward<input type="number" min="1" value={tokenForm.miningReward} onChange={(event) => setTokenForm({ ...tokenForm, miningReward: event.target.value })} /></label> : <label>Initial issuance<input type="number" min="0" value={tokenForm.initialIssuance} onChange={(event) => setTokenForm({ ...tokenForm, initialIssuance: event.target.value })} /></label>}
                  <button className="button primary" type="submit" disabled={disabled || !state?.walletId}><Sparkles size={15} /> Create token</button>
                </form>
              </section>
            </>
          )}
          {page === "Settings" && (
            <section className="panel form-panel settings-panel">
              <h2>Workspace settings</h2>
              <p>
                Stop mining before changing connection or hardware settings.
              </p>
              <div className="two-col">
                <label>
                  PengChain server URL
                  <input
                    type="url"
                    value={draft.server}
                    disabled={disabled || running}
                    onChange={(e) => update("server", e.target.value)}
                  />
                  <span className="small muted">
                    HTTPS required for remote servers. Loopback HTTP is allowed
                    for local development.
                  </span>
                </label>
                <label>
                  Default token
                  <input
                    value={draft.token}
                    maxLength={16}
                    disabled={disabled || running}
                    onChange={(e) => update("token", e.target.value)}
                  />
                </label>
                <div>{selector}</div>
                <div className="preferences">
                  <label>
                    Theme
                    <select
                      value={draft.theme}
                      disabled={disabled || running}
                      onChange={(e) => update("theme", e.target.value)}
                    >
                      <option value="dark">Charcoal dark</option>
                      <option value="light">Light</option>
                      <option value="system">Follow system</option>
                    </select>
                  </label>
                  {(
                    [
                      [
                        "startOnLaunch",
                        "Start mining on launch",
                        "Opt-in only. Requires a valid saved session.",
                      ],
                      [
                        "minimizeToTray",
                        "Close to system tray",
                        "Keep mining when the window is closed. Quit from the tray to stop.",
                      ],
                      [
                        "notifications",
                        "Desktop notifications",
                        "Notify on accepted rewards and mining errors.",
                      ],
                    ] as const
                  ).map(([key, title, detail]) => (
                    <label className="toggle-row" key={key}>
                      <span>
                        <strong>{title}</strong>
                        <small>{detail}</small>
                      </span>
                      <input
                        type="checkbox"
                        checked={draft[key]}
                        disabled={disabled || running}
                        onChange={(e) => update(key, e.target.checked)}
                      />
                    </label>
                  ))}
                </div>
              </div>
              <button
                className="button primary"
                disabled={disabled || running}
                onClick={() => void act("save_settings", { settings: draft })}
              >
                <Check size={16} /> Save settings
              </button>
            </section>
          )}
          {page === "Logs" && (
            <section className="panel">
              <div className="panel-head">
                <div>
                  <h2>Activity log</h2>
                  <p>
                    Server audit history plus local runtime events. No passwords
                    or session cookies.
                  </p>
                </div>
                <Activity size={18} />
              </div>
              <div className="logs">
                {[...(state?.logs ?? [])].reverse().map((log, i) => (
                  <div className="log-row" key={`${log.time}-${i}`}>
                    <time>{time(log.time)}</time>
                    <span
                      className={`log-kind ${log.kind.includes("error") ? "red" : ""}`}
                    >
                      {log.kind.replaceAll("_", " ")}
                    </span>
                    <p>{log.message}</p>
                  </div>
                ))}
                {!state?.logs.length && (
                  <div className="empty">No activity yet.</div>
                )}
              </div>
              <div className="panel-head">
                <div>
                  <h2>Server audit history</h2>
                  <p>Hash-linked records from the authoritative ledger.</p>
                </div>
                <ShieldCheck size={18} />
              </div>
              <div className="logs">
                {(state?.auditLogs ?? []).map((log) => (
                  <div className="log-row" key={log.id}>
                    <time>{time(log.createdAt)}</time>
                    <span className="log-kind">
                      {log.eventType.replaceAll(".", " ")}
                    </span>
                    <p>{log.payload}</p>
                  </div>
                ))}
                {!state?.auditLogs.length && (
                  <div className="empty">No server audit records yet.</div>
                )}
              </div>
            </section>
          )}
          <footer>
            <span>
              <ShieldCheck size={13} /> Server-authoritative ledger · PCA-10
            </span>
            <span>
              PikaStudio <span className="footer-dot">/</span> PengMiner 1.0.0
            </span>
          </footer>
        </div>
      </main>
    </div>
  );
}
