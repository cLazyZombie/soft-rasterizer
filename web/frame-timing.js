export const FRAME_TIMING_CAPACITY = 120;
export const FRAME_RATE_CAPACITY = 20;

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

export class FrameRateTracker {
  #capacity;
  #intervals = [];
  #next = 0;
  #previousTimestamp = null;
  #totalIntervalMs = 0;

  constructor(capacity = FRAME_RATE_CAPACITY) {
    if (!Number.isInteger(capacity) || capacity <= 0) {
      throw new Error("frame rate tracker capacity는 양의 정수여야 합니다");
    }
    this.#capacity = capacity;
  }

  pushTimestamp(timestampMs) {
    if (!Number.isFinite(timestampMs) || timestampMs < 0) {
      throw new Error("frame timestamp는 유한한 0 이상이어야 합니다");
    }
    if (this.#previousTimestamp === null) {
      this.#previousTimestamp = timestampMs;
      return this.summary();
    }
    if (timestampMs <= this.#previousTimestamp) {
      throw new Error("frame timestamp는 이전 값보다 커야 합니다");
    }

    const intervalMs = timestampMs - this.#previousTimestamp;
    this.#previousTimestamp = timestampMs;
    if (this.#intervals.length < this.#capacity) {
      this.#intervals.push(intervalMs);
      this.#totalIntervalMs += intervalMs;
      return this.summary();
    }

    this.#totalIntervalMs -= this.#intervals[this.#next];
    this.#intervals[this.#next] = intervalMs;
    this.#totalIntervalMs += intervalMs;
    this.#next = (this.#next + 1) % this.#capacity;
    return this.summary();
  }

  reset() {
    this.#intervals = [];
    this.#next = 0;
    this.#previousTimestamp = null;
    this.#totalIntervalMs = 0;
  }

  summary() {
    return {
      count: this.#intervals.length,
      fps:
        this.#intervals.length === 0
          ? null
          : (1000 * this.#intervals.length) / this.#totalIntervalMs,
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
