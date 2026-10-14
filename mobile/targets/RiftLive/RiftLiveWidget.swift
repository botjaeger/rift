// The Live Activity of a command running on a Rift computer (the P6 Lock board): the command,
// where it runs, a clock the system ticks on its own (Text(timerInterval:)), its latest line;
// once it finished, its result.
import ActivityKit
import SwiftUI
import WidgetKit

private let accent = Color(red: 0x56 / 255, green: 0x9c / 255, blue: 0xd6 / 255)
private let success = Color(red: 0x22 / 255, green: 0xc5 / 255, blue: 0x5e / 255)
private let bright = Color(red: 0xf0 / 255, green: 0xf1 / 255, blue: 0xf3 / 255)
private let muted = Color(red: 0xa1 / 255, green: 0xa4 / 255, blue: 0xad / 255)
private let ground = Color(red: 0x1e / 255, green: 0x1e / 255, blue: 0x1e / 255)
private let tile = Color(red: 0x18 / 255, green: 0x18 / 255, blue: 0x18 / 255)

/// "4:12" while it runs; "✓" or "✕ 101" once it finished; "stopped" when it ended without an exit status (the wait was dropped, or a later launch ended it).
private struct Clock: View {
  let attributes: RiftLiveAttributes
  let state: RiftLiveAttributes.ContentState
  var size: CGFloat = 20

  var body: some View {
    if state.finishedAt != nil {
      let failed = (state.exit ?? 0) != 0
      Text(failed ? "✕ \(state.exit!)" : (state.exit == nil ? "stopped" : "✓"))
        .font(.system(size: size, design: .monospaced))
        .foregroundColor(failed ? bright : state.exit == nil ? muted : success)
    } else {
      Text(timerInterval: attributes.startedAt...Date.distantFuture, countsDown: false)
        .font(.system(size: size, design: .monospaced))
        .monospacedDigit()
        .foregroundColor(accent)
        .multilineTextAlignment(.trailing)
        .frame(maxWidth: size * 4.2, alignment: .trailing)
    }
  }
}

private struct Mark: View {
  var body: some View {
    Text("❯")
      .font(.system(size: 13, design: .monospaced))
      .foregroundColor(accent)
      .frame(width: 26, height: 26)
      .background(RoundedRectangle(cornerRadius: 7).fill(tile))
      .overlay(RoundedRectangle(cornerRadius: 7).stroke(Color.white.opacity(0.14), lineWidth: 1))
  }
}

private struct LockScreen: View {
  let context: ActivityViewContext<RiftLiveAttributes>

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 10) {
        Mark()
        VStack(alignment: .leading, spacing: 1) {
          Text(context.attributes.command)
            .font(.system(size: 14, design: .monospaced))
            .foregroundColor(bright)
            .lineLimit(1)
          Text(context.attributes.place)
            .font(.system(size: 12))
            .foregroundColor(muted)
            .lineLimit(1)
        }
        Spacer(minLength: 8)
        Clock(attributes: context.attributes, state: context.state)
      }
      if !context.state.line.isEmpty {
        Text(context.state.line)
          .font(.system(size: 12, design: .monospaced))
          .foregroundColor(muted)
          .lineLimit(1)
      }
    }
    .padding(.vertical, 14)
    .padding(.horizontal, 16)
  }
}

struct RiftLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: RiftLiveAttributes.self) { context in
      LockScreen(context: context)
        .activityBackgroundTint(ground.opacity(0.95))
        .activitySystemActionForegroundColor(bright)
    } dynamicIsland: { context in
      DynamicIsland {
        DynamicIslandExpandedRegion(.leading) { Mark() }
        DynamicIslandExpandedRegion(.trailing) { Clock(attributes: context.attributes, state: context.state, size: 18) }
        DynamicIslandExpandedRegion(.center) {
          Text(context.attributes.command)
            .font(.system(size: 14, design: .monospaced))
            .foregroundColor(bright)
            .lineLimit(1)
        }
        DynamicIslandExpandedRegion(.bottom) {
          Text(context.state.line.isEmpty ? context.attributes.place : context.state.line)
            .font(.system(size: 12, design: .monospaced))
            .foregroundColor(muted)
            .lineLimit(1)
        }
      } compactLeading: {
        Text("❯").font(.system(size: 13, design: .monospaced)).foregroundColor(accent)
      } compactTrailing: {
        Clock(attributes: context.attributes, state: context.state, size: 13)
      } minimal: {
        Text("❯").font(.system(size: 13, design: .monospaced)).foregroundColor(accent)
      }
    }
  }
}

@main
struct RiftLiveBundle: WidgetBundle {
  var body: some Widget {
    RiftLiveActivity()
  }
}
