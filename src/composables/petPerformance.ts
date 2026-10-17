export interface PetTiming {
  startedAt: number;
  readyMs?: number;
  dispatchMs?: number;
  firstAnswerMs?: number;
  firstPaintMs?: number;
  completedMs?: number;
  toolCalls: number;
}
export type PetTimingStage = Exclude<keyof PetTiming, "startedAt" | "toolCalls">;

export function startPetTiming(now = Date.now()): PetTiming {
  return {startedAt: now, toolCalls: 0};
}
export function markPetTiming(timing: PetTiming, stage: PetTimingStage, now = Date.now()) {
  if (timing[stage] !== undefined) return;
  if (stage === "firstPaintMs" && timing.firstAnswerMs === undefined) return;
  timing[stage] = Math.max(0, now - timing.startedAt);
}
