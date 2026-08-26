export const FRAME_TIMING_CAPACITY = 120;

function percentileNearestRank(values, percentile) {
  if (values.length === 0) {
    return null;
  }
  const sorted = values.toSorted((left, right) => left - right);
  const rank = Math.ceil(percentile * sorted.length);
  return sorted[Math.max(0, rank - 1)];
}

function summarizeField(samples, field) {
  const values = samples.map((sample) => sample[field]);
  return {
    p50: percentileNearestRank(values, 0.5),
    p95: percentileNearestRank(values, 0.95),
  };
}

export class FrameTimingRing {
  #capacity;
  #samples = [];
  #next = 0;

  constructor(capacity = FRAME_TIMING_CAPACITY) {
    if (!Number.isInteger(capacity) || capacity <= 0) {
      throw new Error("frame timing ring capacity는 양의 정수여야 합니다");
    }
    this.#capacity = capacity;
  }

  push(sample) {
    for (const field of ["updateMs", "presentMs", "totalMs"]) {
      if (!Number.isFinite(sample[field]) || sample[field] < 0) {
        throw new Error(`frame timing ${field}는 유한한 0 이상이어야 합니다`);
      }
    }
    if (this.#samples.length < this.#capacity) {
      this.#samples.push(sample);
      return;
    }
    this.#samples[this.#next] = sample;
    this.#next = (this.#next + 1) % this.#capacity;
  }

  summary() {
    return {
      count: this.#samples.length,
      updateMs: summarizeField(this.#samples, "updateMs"),
      presentMs: summarizeField(this.#samples, "presentMs"),
      totalMs: summarizeField(this.#samples, "totalMs"),
    };
  }
}

export function summarizeFrameTimings(samples) {
  const ring = new FrameTimingRing(samples.length);
  for (const sample of samples) {
    ring.push(sample);
  }
  return ring.summary();
}
