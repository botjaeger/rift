// The Live Activity's data, the same in the app (modules/rift-live, a copy the plugin keeps in
// step) and in the widget extension (targets/RiftLive): ActivityKit matches them by name and
// shape.
import ActivityKit
import Foundation

public struct RiftLiveAttributes: ActivityAttributes {
  public struct ContentState: Codable, Hashable {
    /// The command's latest output line ("" when unknown).
    public var line: String
    /// Set once it finished: its exit status (nil: unknown) and when.
    public var exit: Int?
    public var finishedAt: Date?

    public init(line: String, exit: Int? = nil, finishedAt: Date? = nil) {
      self.line = line
      self.exit = exit
      self.finishedAt = finishedAt
    }
  }

  /// "cargo test --workspace"
  public var command: String
  /// "studio-mac · rift"
  public var place: String
  /// When it started (the computer's clock, or when this phone asked).
  public var startedAt: Date

  public init(command: String, place: String, startedAt: Date) {
    self.command = command
    self.place = place
    self.startedAt = startedAt
  }
}
