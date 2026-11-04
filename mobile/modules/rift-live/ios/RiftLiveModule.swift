import ActivityKit
import ExpoModulesCore

/// Starts, updates and ends the Live Activity of a command running on a Rift computer (the P6
/// Lock board). Only while the app runs: updates and the end after the app was suspended would
/// need ActivityKit push, which this build has no APNs entitlement for (mobile/README.md › Push).
public class RiftLiveModule: Module {
  /// Shown as out of date this long after its last update: an app that was closed or killed
  /// updates it no more (the next launch ends it).
  private static func staleDate() -> Date { Date().addingTimeInterval(60 * 60) }

  @available(iOS 16.2, *)
  private static func end(_ id: String, _ state: RiftLiveAttributes.ContentState) async {
    for activity in Activity<RiftLiveAttributes>.activities where activity.id == id {
      // the finished state stays on the lock screen a while, as Apple's own timers do
      await activity.end(ActivityContent(state: state, staleDate: nil), dismissalPolicy: .after(Date().addingTimeInterval(15 * 60)))
    }
  }

  public func definition() -> ModuleDefinition {
    Name("RiftLive")

    Function("isSupported") { () -> Bool in
      if #available(iOS 16.2, *) {
        return ActivityAuthorizationInfo().areActivitiesEnabled
      }
      return false
    }

    AsyncFunction("start") { (command: String, place: String, startedAtMs: Double, line: String) -> String? in
      guard #available(iOS 16.2, *), ActivityAuthorizationInfo().areActivitiesEnabled else { return nil }
      let attributes = RiftLiveAttributes(
        command: command, place: place, startedAt: Date(timeIntervalSince1970: startedAtMs / 1000))
      let content = ActivityContent(state: RiftLiveAttributes.ContentState(line: line), staleDate: Self.staleDate())
      let activity = try Activity.request(attributes: attributes, content: content, pushType: nil)
      return activity.id
    }

    AsyncFunction("startLinked") { (command: String, place: String, link: String, startedAtMs: Double, line: String) -> String? in
      guard #available(iOS 16.2, *), ActivityAuthorizationInfo().areActivitiesEnabled else { return nil }
      let attributes = RiftLiveAttributes(
        command: command, place: place, startedAt: Date(timeIntervalSince1970: startedAtMs / 1000), link: link)
      let content = ActivityContent(state: RiftLiveAttributes.ContentState(line: line), staleDate: Self.staleDate())
      let activity = try Activity.request(attributes: attributes, content: content, pushType: nil)
      return activity.id
    }

    AsyncFunction("update") { (id: String, line: String) in
      guard #available(iOS 16.2, *) else { return }
      for activity in Activity<RiftLiveAttributes>.activities where activity.id == id {
        await activity.update(ActivityContent(state: .init(line: line), staleDate: Self.staleDate()))
      }
    }

    AsyncFunction("end") { (id: String, exit: Int?, line: String) in
      guard #available(iOS 16.2, *) else { return }
      await Self.end(id, RiftLiveAttributes.ContentState(line: line, exit: exit, finishedAt: Date()))
    }

    // one that finished while the phone was away: when is not known, so no duration
    AsyncFunction("endUntimed") { (id: String, exit: Int?, line: String) in
      guard #available(iOS 16.2, *) else { return }
      await Self.end(id, RiftLiveAttributes.ContentState(line: line, exit: exit, finishedAt: Date(), timed: false))
    }

    AsyncFunction("ids") { () -> [String] in
      guard #available(iOS 16.2, *) else { return [] }
      return Activity<RiftLiveAttributes>.activities.map { $0.id }
    }
  }
}
