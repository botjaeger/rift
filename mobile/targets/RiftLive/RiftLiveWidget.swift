// The Live Activity of a command running on a Rift computer (the P6 Lock board): the command,
// where it runs, a clock the system ticks on its own (Text(timerInterval:)), its latest line;
// once it finished, its result.
import ActivityKit
import SwiftUI
import WidgetKit

private let accent = Color(red: 0x19 / 255, green: 0xaa / 255, blue: 0xd8 / 255)
private let success = Color(red: 0x1c / 255, green: 0xa0 / 255, blue: 0x5a / 255)
private let bright = Color(red: 0xff / 255, green: 0xff / 255, blue: 0xff / 255)
private let muted = Color(red: 0x9b / 255, green: 0x9b / 255, blue: 0x9b / 255)
private let ground = Color(red: 0x05 / 255, green: 0x05 / 255, blue: 0x05 / 255)

private let danger = Color(red: 0xff / 255, green: 0x82 / 255, blue: 0x72 / 255)

/// "113ms", "8.4s", "12s", "2m 5s", "1h 2m": the words mobile/src/notifyText.ts `durationText` and
/// crates/rift-remote/src/push.rs `duration_text` use.
private func durationText(_ interval: TimeInterval) -> String {
  let ms = Int((max(0, interval) * 1000).rounded())
  if ms < 1000 { return "\(ms)ms" }
  let tenths = ms / 100
  if tenths < 100 { return "\(tenths / 10).\(tenths % 10)s" }
  let s = ms / 1000
  let h = s / 3600
  let m = s / 60 % 60
  if h > 0 { return "\(h)h \(m)m" }
  if m > 0 { return "\(m)m \(s % 60)s" }
  return "\(s)s"
}

/// The clock while it runs ("4:12", ticked by the system); once it finished, a status badge:
/// "✓ 0" (green), "✕ 101" (red), "stopped" (muted: it ended without an exit status, the wait was
/// dropped or a later launch ended it).
private struct Clock: View {
  let attributes: RiftLiveAttributes
  let state: RiftLiveAttributes.ContentState
  var size: CGFloat = 20
  /// No update for its stale hour: the app is closed or suspended, so the result cannot come in
  /// until it is opened. A clock ticking on would say the command still runs.
  var stale = false

  var body: some View {
    if state.finishedAt != nil {
      Badge(exit: state.exit, size: size)
    } else if stale {
      Text("Open Rift")
        .font(.system(size: size * 0.7, weight: .medium))
        .foregroundColor(muted)
        .lineLimit(1)
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

private struct Badge: View {
  let exit: Int?
  var size: CGFloat = 14

  var body: some View {
    let tint: Color = exit == nil ? muted : exit == 0 ? success : danger
    Text(exit == nil ? "stopped" : exit == 0 ? "✓ 0" : "✕ \(exit!)")
      .font(.system(size: size * 0.8, weight: .semibold, design: .monospaced))
      .foregroundColor(tint)
      .lineLimit(1)
      .padding(.horizontal, size * 0.55)
      .padding(.vertical, size * 0.25)
      .background(Capsule().fill(tint.opacity(0.16)))
  }
}

/// "Finished in 113ms" / "Failed after 8.4s" (nil while it runs or when it has no end).
private func resultText(_ attributes: RiftLiveAttributes, _ state: RiftLiveAttributes.ContentState) -> String? {
  guard let end = state.finishedAt else { return nil }
  // finished while the phone was away: how long it took is not known
  if state.timed == false {
    guard let exit = state.exit else { return "Stopped" }
    return exit == 0 ? "Finished" : "Failed"
  }
  let took = durationText(end.timeIntervalSince(attributes.startedAt))
  guard let exit = state.exit else { return "Stopped after \(took)" }
  return exit == 0 ? "Finished in \(took)" : "Failed after \(took)"
}

/// The Rift mark (mobile/assets/icon.png, 1024 units): a blue rounded chevron and a light
/// rounded cursor on #232326, drawn here with the same geometry.
private struct Mark: View {
  var size: CGFloat = 34
  var radius: CGFloat = 8

  var body: some View {
    Canvas { ctx, canvas in
      let k = canvas.width / 1024
      ctx.fill(Path(CGRect(origin: .zero, size: canvas)), with: .color(Color(red: 0x23 / 255, green: 0x23 / 255, blue: 0x26 / 255)))
      var chevron = Path()
      chevron.move(to: CGPoint(x: 293 * k, y: 324 * k))
      chevron.addLine(to: CGPoint(x: 503 * k, y: 511 * k))
      chevron.addLine(to: CGPoint(x: 293 * k, y: 700 * k))
      ctx.stroke(chevron, with: .color(accent), style: StrokeStyle(lineWidth: 124 * k, lineCap: .round, lineJoin: .round))
      let cursor = CGRect(x: 616 * k, y: 338 * k, width: 174 * k, height: 348 * k)
      ctx.fill(Path(roundedRect: cursor, cornerRadius: 20 * k), with: .color(Color(red: 0xd4 / 255, green: 0xd4 / 255, blue: 0xd4 / 255)))
    }
    .frame(width: size, height: size)
    .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
    .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).stroke(Color.white.opacity(0.12), lineWidth: 1))
  }
}

private struct LockScreen: View {
  let attributes: RiftLiveAttributes
  let state: RiftLiveAttributes.ContentState
  var stale = false

  var body: some View {
    let result = resultText(attributes, state)
    let failed = (state.exit ?? 0) != 0
    VStack(alignment: .leading, spacing: 8) {
      HStack(spacing: 10) {
        Mark()
        VStack(alignment: .leading, spacing: 2) {
          Text(attributes.command)
            .font(.system(size: 14, design: .monospaced))
            .foregroundColor(bright)
            .lineLimit(1)
          Text(attributes.place)
            .font(.system(size: 12))
            .foregroundColor(muted)
            .lineLimit(1)
        }
        Spacer(minLength: 8)
        Clock(attributes: attributes, state: state, stale: stale)
      }
      if let result {
        Text(result)
          .font(.system(size: 12, weight: .medium))
          .foregroundColor(state.exit == nil ? muted : failed ? danger : success)
          .lineLimit(1)
      }
      if !state.line.isEmpty {
        Text(state.line)
          .font(.system(size: 12, design: .monospaced))
          .foregroundColor(muted)
          .lineLimit(1)
      }
    }
    .padding(.vertical, 14)
    .padding(.horizontal, 16)
    .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).stroke(Color.white.opacity(0.12), lineWidth: 1))
  }
}

/// The pane's screen in the app (nil: an activity started before links, which opens the app).
private func paneURL(_ attributes: RiftLiveAttributes) -> URL? {
  attributes.link.flatMap { URL(string: $0) }
}

struct RiftLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: RiftLiveAttributes.self) { context in
      LockScreen(attributes: context.attributes, state: context.state, stale: context.isStale)
        .activityBackgroundTint(ground.opacity(0.95))
        .activitySystemActionForegroundColor(bright)
        // a tap opens the pane it is about (where its bell turns the wait off)
        .widgetURL(paneURL(context.attributes))
    } dynamicIsland: { context in
      DynamicIsland {
        DynamicIslandExpandedRegion(.leading) { Mark(size: 28, radius: 7) }
        DynamicIslandExpandedRegion(.trailing) {
          Clock(attributes: context.attributes, state: context.state, size: 18, stale: context.isStale)
        }
        DynamicIslandExpandedRegion(.center) {
          VStack(spacing: 2) {
            Text(context.attributes.command)
              .font(.system(size: 14, design: .monospaced))
              .foregroundColor(bright)
              .lineLimit(1)
            // which computer, tab and pane it runs in
            Text(context.attributes.place)
              .font(.system(size: 12))
              .foregroundColor(muted)
              .lineLimit(1)
          }
        }
        DynamicIslandExpandedRegion(.bottom) {
          if let text = resultText(context.attributes, context.state) ?? (context.state.line.isEmpty ? nil : context.state.line) {
            Text(text)
              .font(.system(size: 12, design: .monospaced))
              .foregroundColor(muted)
              .lineLimit(1)
          }
        }
      } compactLeading: {
        Mark(size: 20, radius: 5)
      } compactTrailing: {
        Clock(attributes: context.attributes, state: context.state, size: 13, stale: context.isStale)
      } minimal: {
        Mark(size: 20, radius: 5)
      }
      .widgetURL(paneURL(context.attributes))
    }
  }
}

@main
struct RiftLiveBundle: WidgetBundle {
  var body: some Widget {
    RiftLiveActivity()
  }
}
