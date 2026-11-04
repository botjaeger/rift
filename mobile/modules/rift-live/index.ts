// The lock screen's Live Activity for a running command (iOS 16.2+; ios/RiftLiveModule.swift and
// the widget in targets/RiftLive). Elsewhere, or in a build without the module, nothing happens.
import { requireOptionalNativeModule } from "expo";

interface RiftLiveNative {
  isSupported(): boolean;
  start(command: string, place: string, startedAtMs: number, line: string): Promise<string | null>;
  /** `start` with the pane's link, which a tap on the activity opens (absent in builds before it). */
  startLinked?(command: string, place: string, link: string, startedAtMs: number, line: string): Promise<string | null>;
  update(id: string, line: string): Promise<void>;
  end(id: string, exit: number | null, line: string): Promise<void>;
  /** `end` for one whose end time is unknown: no duration shown (absent in builds before it). */
  endUntimed?(id: string, exit: number | null, line: string): Promise<void>;
  ids(): Promise<string[]>;
}

const native = requireOptionalNativeModule<RiftLiveNative>("RiftLive");

export const liveActivities = {
  supported: (): boolean => {
    try {
      return native?.isSupported() ?? false;
    } catch {
      return false;
    }
  },
  start: (command: string, place: string, link: string, startedAtMs: number, line: string) =>
    !native
      ? Promise.resolve(null)
      : (typeof native.startLinked === "function"
          ? native.startLinked(command, place, link, startedAtMs, line)
          : native.start(command, place, startedAtMs, line)
        ).catch(() => null),
  update: (id: string, line: string) => (native ? native.update(id, line).catch(() => undefined) : Promise.resolve()),
  end: (id: string, exit: number | null, line: string, timed = true) =>
    !native
      ? Promise.resolve()
      : (!timed && typeof native.endUntimed === "function" ? native.endUntimed(id, exit, line) : native.end(id, exit, line)).catch(
          () => undefined,
        ),
  ids: () => (native ? native.ids().catch(() => []) : Promise.resolve([])),
};
