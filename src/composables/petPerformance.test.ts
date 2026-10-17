import { describe, expect, it } from "vitest";
import { startPetTiming, markPetTiming } from "./petPerformance";

describe("pet latency measurements", () => {
  it("measures startup, dispatch, first answer and first paint separately", () => {
    const timing = startPetTiming(1000);
    markPetTiming(timing, "readyMs", 1100);
    markPetTiming(timing, "dispatchMs", 1200);
    markPetTiming(timing, "firstPaintMs", 1300); // no answer yet
    expect(timing.firstPaintMs).toBeUndefined();
    markPetTiming(timing, "firstAnswerMs", 1400);
    markPetTiming(timing, "firstAnswerMs", 1500); // later chunks do not reset it
    markPetTiming(timing, "firstPaintMs", 1420);
    expect(timing).toMatchObject({readyMs:100, dispatchMs:200, firstAnswerMs:400, firstPaintMs:420});
  });
  it("does not carry a previous turn's measurements into the next", () => {
    const timing = startPetTiming(1000);
    markPetTiming(timing, "firstAnswerMs", 1010);
    expect(startPetTiming(2000).firstAnswerMs).toBeUndefined();
  });
});
