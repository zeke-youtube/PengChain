export const MIN_DIFFICULTY = 1;
export const MAX_DIFFICULTY = 64;
export const ADJUSTMENT_WINDOW = 10;
export const FAST_INTERVAL_SECONDS = 30;
export const SLOW_INTERVAL_SECONDS = 120;
export const STALL_INTERVAL_SECONDS = 60;

export type DifficultyAdjustment = {
  difficulty: number;
  averageIntervalSeconds: number;
  direction: "increase" | "decrease" | "unchanged";
};

export function emergencyDifficulty(
  currentDifficulty: number,
  lastAcceptedAt: number | null,
  currentTime: number,
): number {
  if (
    lastAcceptedAt !== null &&
    currentTime - lastAcceptedAt < STALL_INTERVAL_SECONDS * 1000
  ) {
    return currentDifficulty;
  }
  return Math.max(MIN_DIFFICULTY, currentDifficulty - 1);
}

export function adjustDifficulty(
  currentDifficulty: number,
  acceptedAt: readonly number[],
): DifficultyAdjustment {
  if (acceptedAt.length !== ADJUSTMENT_WINDOW) {
    return {
      difficulty: currentDifficulty,
      averageIntervalSeconds: 0,
      direction: "unchanged",
    };
  }

  const first = acceptedAt[0];
  const last = acceptedAt[acceptedAt.length - 1];
  const averageIntervalSeconds = Math.max(0, (last - first) / 1000 / 9);

  if (averageIntervalSeconds < FAST_INTERVAL_SECONDS) {
    return {
      difficulty: Math.min(MAX_DIFFICULTY, currentDifficulty + 1),
      averageIntervalSeconds,
      direction: "increase",
    };
  }

  if (averageIntervalSeconds > SLOW_INTERVAL_SECONDS) {
    return {
      difficulty: Math.max(MIN_DIFFICULTY, currentDifficulty - 1),
      averageIntervalSeconds,
      direction: "decrease",
    };
  }

  return {
    difficulty: currentDifficulty,
    averageIntervalSeconds,
    direction: "unchanged",
  };
}
