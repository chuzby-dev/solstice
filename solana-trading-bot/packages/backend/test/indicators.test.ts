import { describe, expect, it } from "vitest";
import { bollingerBands, closeToCloseVolatility, emaLatest, macd, rsi, sma } from "../src/strategy-engine/indicators.js";

describe("sma", () => {
  it("averages the last `period` values", () => {
    expect(sma([1, 2, 3, 4, 5], 3)).toBeCloseTo(4, 10); // (3+4+5)/3
  });

  it("returns null when there isn't enough data", () => {
    expect(sma([1, 2], 3)).toBeNull();
  });
});

describe("emaLatest", () => {
  it("stays at the constant value for a flat series", () => {
    expect(emaLatest(Array(30).fill(50), 12)).toBeCloseTo(50, 10);
  });

  it("returns null when there isn't enough data", () => {
    expect(emaLatest([1, 2, 3], 12)).toBeNull();
  });
});

describe("rsi", () => {
  it("is 100 for a strictly increasing series (no losses)", () => {
    const values = Array.from({ length: 15 }, (_, i) => i + 1); // [1..15]
    expect(rsi(values, 14)).toBe(100);
  });

  it("is 0 for a strictly decreasing series (no gains)", () => {
    const values = Array.from({ length: 15 }, (_, i) => 15 - i); // [15..1]
    expect(rsi(values, 14)).toBe(0);
  });

  it("returns null when there isn't enough data", () => {
    expect(rsi([1, 2, 3], 14)).toBeNull();
  });
});

describe("macd", () => {
  it("returns null when there isn't enough data", () => {
    expect(macd(Array(30).fill(100), 12, 26, 9)).toBeNull(); // needs 26 + 9 = 35
  });

  it("is all zero for a flat series (fast EMA == slow EMA)", () => {
    const result = macd(Array(40).fill(100), 12, 26, 9);
    expect(result).not.toBeNull();
    expect(result!.macdLine).toBeCloseTo(0, 10);
    expect(result!.signalLine).toBeCloseTo(0, 10);
    expect(result!.histogram).toBeCloseTo(0, 10);
  });
});

describe("closeToCloseVolatility", () => {
  it("averages absolute tick-to-tick changes over the window", () => {
    // window of last 5 values: [10,12,9,11,10] -> diffs |2|,|3|,|2|,|1| -> sum 8 / period 4
    expect(closeToCloseVolatility([10, 12, 9, 11, 10], 4)).toBeCloseTo(2, 10);
  });

  it("returns null when there isn't enough data", () => {
    expect(closeToCloseVolatility([1, 2], 4)).toBeNull();
  });
});

describe("bollingerBands", () => {
  it("computes mean and population-stddev-based bands", () => {
    // [10,12,14,16,18] -> mean 14, population variance 8, stdDev ~2.8284
    const result = bollingerBands([10, 12, 14, 16, 18], 5, 2);
    expect(result).not.toBeNull();
    expect(result!.middle).toBeCloseTo(14, 10);
    expect(result!.stdDev).toBeCloseTo(2.8284271247461903, 10);
    expect(result!.upper).toBeCloseTo(19.656854249492378, 10);
    expect(result!.lower).toBeCloseTo(8.343145750507622, 10);
  });

  it("collapses to a zero-width band for a flat series", () => {
    const result = bollingerBands(Array(20).fill(50), 20);
    expect(result?.middle).toBe(50);
    expect(result?.stdDev).toBe(0);
    expect(result?.upper).toBe(50);
    expect(result?.lower).toBe(50);
  });

  it("returns null when there isn't enough data", () => {
    expect(bollingerBands([1, 2, 3], 5)).toBeNull();
  });
});
