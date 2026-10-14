// The lock screen's Live Activity for a running command (iOS 16.2+; ios/RiftLiveModule.swift and
// the widget in targets/RiftLive). Elsewhere, or in a build without the module, nothing happens.
import { requireOptionalNativeModule } from "expo";

interface RiftLiveNative {
  isSupported(): boolean;
  start(command: string, place: string, startedAtMs: number, line: string): Promise<string | null>;
  update(id: string, line: string): Promise<void>;
  end(id: string, exit: number | null, line: string): Promise<void>;
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
  start: (command: string, place: string, startedAtMs: number, line: string) =>
    native ? native.start(command, place, startedAtMs, line).catch(() => null) : Promise.resolve(null),
  update: (id: string, line: string) => (native ? native.update(id, line).catch(() => undefined) : Promise.resolve()),
  end: (id: string, exit: number | null, line: string) => (native ? native.end(id, exit, line).catch(() => undefined) : Promise.resolve()),
  ids: () => (native ? native.ids().catch(() => []) : Promise.resolve([])),
};
