import ExpoModulesCore

public class RiftKeysModule: Module {
  public func definition() -> ModuleDefinition {
    Name("RiftKeys")

    View(KeyCaptureView.self) {
      Events("onText", "onKey", "onFocusChange", "onPaste")

      Prop("mode") { (view: KeyCaptureView, mode: String) in
        view.mode = mode == "terminal" ? .terminal : .shortcuts
      }

      Prop("captureTab") { (view: KeyCaptureView, capture: Bool) in
        view.captureTab = capture
      }

      AsyncFunction("focus") { (view: KeyCaptureView) in
        view.focusCapture()
      }.runOnQueue(.main)

      AsyncFunction("blur") { (view: KeyCaptureView) in
        view.blurCapture()
      }.runOnQueue(.main)
    }
  }
}
