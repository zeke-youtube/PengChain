import { describe, expect, it } from "vitest";
import {
  adjustDifficulty,
  MAX_DIFFICULTY,
  MIN_DIFFICULTY,
} from "../src/difficulty.js";

function timestamps(intervalSeconds: number): number[] {
  return Array.from(
    { length: 10 },
    (_, index) => index * intervalSeconds * 1000,
  );
}

describe("adaptive mining difficulty", () => {
  it("increases after a fast ten-solution window", () => {
    const result = adjustDifficulty(3, timestamps(10));
    expect(result).toMatchObject({
      difficulty: 4,
      direction: "increase",
    });
  });

  it("decreases after a slow ten-solution window", () => {
    const result = adjustDifficulty(3, timestamps(150));
    expect(result).toMatchObject({
      difficulty: 2,
      direction: "decrease",
    });
  });

  it("keeps difficulty unchanged inside the target range", () => {
    const result = adjustDifficulty(3, timestamps(60));
    expect(result).toMatchObject({
      difficulty: 3,
      direction: "unchanged",
    });
  });

  it("does not adjust before ten accepted solutions", () => {
    expect(adjustDifficulty(3, timestamps(10).slice(0, 9)).difficulty).toBe(3);
  });

  it("clamps at the allowed bounds", () => {
    expect(adjustDifficulty(MAX_DIFFICULTY, timestamps(10)).difficulty).toBe(
      MAX_DIFFICULTY,
    );
    expect(adjustDifficulty(MIN_DIFFICULTY, timestamps(150)).difficulty).toBe(
      MIN_DIFFICULTY,
    );
  });
});
